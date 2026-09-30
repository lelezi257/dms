//! Per-workspace serialized export operations with conservative failure state.

use super::{
    MountIdentity, NamespaceIdentity, NativeDesiredState, NativeState, NativeStatus,
    WorkspaceIdentity, WorkspaceMount,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex, MutexGuard},
};

/// A backend owns and checks prepared directory descriptors. It must report
/// actual namespace/mount/inode identities and perform only normal unmount.
/// Returning an error does not imply that a mount syscall had no side effect.
pub trait MountBackend: Send + Sync {
    fn inspect(&self, spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>>;
    fn bind(&self, spec: &WorkspaceMount) -> io::Result<MountIdentity>;
    /// Return only the identity recorded from this backend's own successful
    /// attach syscall, including when a later verification made bind fail.
    /// Matching an observed source/path is not an ownership proof.
    fn attached_claim(&self, _spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        Ok(None)
    }
    fn unmount(&self, spec: &WorkspaceMount, mount: &MountIdentity) -> io::Result<()>;
}

struct Record {
    spec: WorkspaceMount,
    status: NativeStatus,
    owned_mount: Option<MountIdentity>,
    retired: HashSet<WorkspaceIdentity>,
}

impl Record {
    fn new(spec: WorkspaceMount) -> Self {
        Self {
            status: NativeStatus {
                identity: spec.identity.clone(),
                desired: NativeDesiredState::Native,
                state: NativeState::FuseReady,
                operation_seq: 0,
                observed: None,
                last_errno: None,
                last_error: None,
            },
            spec,
            owned_mount: None,
            retired: HashSet::new(),
        }
    }
    fn check(&self, id: &WorkspaceIdentity) -> io::Result<()> {
        if self.spec.identity != *id {
            return Err(errno(libc::ESTALE));
        }
        Ok(())
    }
    fn advance(&mut self) -> io::Result<()> {
        self.status.operation_seq = self
            .status
            .operation_seq
            .checked_add(1)
            .ok_or_else(|| errno(libc::EOVERFLOW))?;
        Ok(())
    }
    fn fail(&mut self, state: NativeState, error: &io::Error) {
        self.status.state = state;
        self.status.last_errno = error.raw_os_error();
        self.status.last_error = Some(error.to_string());
    }
    fn success(&mut self, state: NativeState) -> NativeStatus {
        self.status.state = state;
        self.status.last_errno = None;
        self.status.last_error = None;
        self.status.clone()
    }
}

type SharedRecord = Arc<Mutex<Record>>;

pub struct NativeMountManager<D: MountBackend> {
    namespace: NamespaceIdentity,
    capacity: usize,
    backend: D,
    records: Mutex<HashMap<String, SharedRecord>>,
}

impl<D: MountBackend> NativeMountManager<D> {
    pub fn new(namespace: NamespaceIdentity, capacity: usize, backend: D) -> io::Result<Self> {
        if namespace.inode == 0 || capacity == 0 || capacity > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid native manager bounds/namespace",
            ));
        }
        Ok(Self {
            namespace,
            capacity,
            backend,
            records: Mutex::new(HashMap::new()),
        })
    }

    /// Admission requires a trusted active Home grant plus prepared descriptors.
    /// This method checks consistency, not the caller's authority in Meta.
    pub fn register(&self, spec: WorkspaceMount) -> io::Result<NativeStatus> {
        if spec.identity.namespace != self.namespace
            || spec.identity.epoch == 0
            || spec.source.inode == 0
            || spec.target.inode == 0
            || [
                &spec.identity.root_id,
                &spec.identity.home_node_id,
                &spec.identity.home_session_id,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 1024)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid prepared native workspace",
            ));
        }
        let old = {
            let mut records = lock(&self.records)?;
            if let Some(old) = records.get(&spec.identity.root_id) {
                old.clone()
            } else {
                if records.len() >= self.capacity {
                    return Err(errno(libc::ENOSPC));
                }
                let record = Record::new(spec);
                let status = record.status.clone();
                records.insert(
                    status.identity.root_id.clone(),
                    Arc::new(Mutex::new(record)),
                );
                return Ok(status);
            }
        };
        let mut old = lock(old.as_ref())?;
        if old.spec == spec {
            return Ok(old.status.clone());
        }
        if old.status.state != NativeState::Detached {
            return Err(errno(libc::EEXIST));
        }
        if old.spec.identity == spec.identity
            || old.retired.contains(&spec.identity)
            || spec.identity.epoch < old.spec.identity.epoch
            || (spec.identity.epoch == old.spec.identity.epoch
                && spec.identity.home_node_id != old.spec.identity.home_node_id)
        {
            return Err(errno(libc::ESTALE));
        }
        if old.retired.len() >= 64 {
            return Err(errno(libc::ENOSPC));
        }
        let previous = old.spec.identity.clone();
        let seq = old
            .status
            .operation_seq
            .checked_add(1)
            .ok_or_else(|| errno(libc::EOVERFLOW))?;
        let mut retired = std::mem::take(&mut old.retired);
        retired.insert(previous);
        *old = Record::new(spec);
        old.retired = retired;
        old.status.operation_seq = seq;
        Ok(old.status.clone())
    }

    pub fn activate(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        if record.status.desired != NativeDesiredState::Native {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native admission is quiesced",
            ));
        }
        let actual = match self.backend.inspect(&record.spec) {
            Ok(actual) => actual,
            Err(error) => {
                record.fail(NativeState::Recovering, &error);
                return Err(error);
            }
        };
        record.status.observed = actual.clone();
        if record.status.state == NativeState::NativeActive {
            if actual.is_some() && actual == record.owned_mount {
                return Ok(record.success(NativeState::NativeActive));
            }
            let error = errno(libc::ESTALE);
            record.fail(NativeState::Recovering, &error);
            return Err(error);
        }
        if !matches!(
            record.status.state,
            NativeState::FuseReady | NativeState::FuseOnly
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native export requires reconciliation",
            ));
        }
        if actual.is_some() {
            let error = errno(libc::EEXIST);
            record.fail(NativeState::Recovering, &error);
            return Err(error);
        }
        record.advance()?;
        record.status.state = NativeState::Mounting;
        match self.backend.bind(&record.spec) {
            Ok(mounted) => {
                record.status.observed = Some(mounted.clone());
                if !mounted.matches(&record.spec) {
                    let error = errno(libc::ESTALE);
                    record.fail(NativeState::Recovering, &error);
                    return Err(error);
                }
                record.owned_mount = Some(mounted);
                Ok(record.success(NativeState::NativeActive))
            }
            Err(error) => {
                match self.backend.inspect(&record.spec) {
                    Ok(None) => {
                        record.status.observed = None;
                        record.fail(NativeState::FuseOnly, &error);
                    }
                    Ok(Some(mounted)) => {
                        if mounted.matches(&record.spec) {
                            match self.backend.attached_claim(&record.spec) {
                                Ok(Some(claim)) if claim == mounted => {
                                    record.owned_mount = Some(claim)
                                }
                                Ok(_) => {}
                                Err(claim_error) => {
                                    record.status.observed = Some(mounted);
                                    record.fail(NativeState::Recovering, &error);
                                    record.status.last_error = Some(format!(
                                        "{error}; attach claim unavailable: {claim_error}"
                                    ));
                                    return Err(error);
                                }
                            }
                        }
                        record.status.observed = Some(mounted);
                        record.fail(NativeState::Recovering, &error);
                    }
                    Err(observation_error) => {
                        record.fail(NativeState::Recovering, &error);
                        record.status.last_error = Some(format!(
                            "{error}; cannot inspect mount: {observation_error}"
                        ));
                    }
                }
                Err(error)
            }
        }
    }

    /// Closes native admission; the caller must independently quiesce/fence
    /// Agents, FUSE handles and peers before workspace authority is changed.
    pub fn quiesce(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        if record.status.desired == NativeDesiredState::Detached {
            return Ok(record.status.clone());
        }
        record.advance()?;
        record.status.desired = NativeDesiredState::Detached;
        record.status.state = NativeState::Quiescing;
        Ok(record.status.clone())
    }

    /// Only removes this manager's verified export with normal umount. Busy
    /// means DRAINING. This does not itself authorize backing deletion/reuse.
    pub fn detach(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        if record.status.state == NativeState::Detached {
            return Ok(record.status.clone());
        }
        if record.status.desired != NativeDesiredState::Detached
            || !matches!(
                record.status.state,
                NativeState::Quiescing | NativeState::Draining
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native export must quiesce before detach",
            ));
        }
        let actual = match self.backend.inspect(&record.spec) {
            Ok(actual) => actual,
            Err(error) => {
                record.fail(NativeState::Draining, &error);
                return Err(error);
            }
        };
        record.status.observed = actual.clone();
        let Some(actual) = actual else {
            if record.owned_mount.is_some() {
                // An external lazy detach may have left live native references.
                let error = errno(libc::ESTALE);
                record.fail(NativeState::Draining, &error);
                return Err(error);
            }
            return Ok(record.success(NativeState::Detached));
        };
        if Some(&actual) != record.owned_mount.as_ref() || !actual.matches(&record.spec) {
            let error = errno(libc::ESTALE);
            record.fail(NativeState::Draining, &error);
            return Err(error);
        }
        record.advance()?;
        if let Err(error) = self.backend.unmount(&record.spec, &actual) {
            record.fail(NativeState::Draining, &error);
            return Err(error);
        }
        match self.backend.inspect(&record.spec) {
            Ok(None) => {
                record.owned_mount = None;
                record.status.observed = None;
                Ok(record.success(NativeState::Detached))
            }
            Ok(Some(mounted)) => {
                record.status.observed = Some(mounted);
                let error = errno(libc::ESTALE);
                record.fail(NativeState::Recovering, &error);
                Err(error)
            }
            Err(error) => {
                record.fail(NativeState::Recovering, &error);
                Err(error)
            }
        }
    }

    pub fn status(&self, root_id: &str) -> io::Result<Option<NativeStatus>> {
        let record = lock(&self.records)?.get(root_id).cloned();
        record
            .map(|record| Ok(lock(&record)?.status.clone()))
            .transpose()
    }

    fn record(&self, root_id: &str) -> io::Result<SharedRecord> {
        lock(&self.records)?
            .get(root_id)
            .cloned()
            .ok_or_else(|| errno(libc::ENOENT))
    }
}

fn errno(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("native mount state is poisoned"))
}
