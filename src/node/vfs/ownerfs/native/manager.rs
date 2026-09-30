//! Per-workspace export transactions and conservative restart reconciliation.
use super::{
    JournalRecord, MountIdentity, MountJournal, NamespaceIdentity, NativeDesiredState, NativeState,
    NativeStatus, WorkspaceIdentity, WorkspaceMount,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex, MutexGuard},
};

/// Implementations use prepared trusted descriptors, inspect current kernel
/// identities and perform only normal unmount in an exclusively managed namespace.
pub trait MountBackend: Send + Sync {
    fn inspect(&self, spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>>;
    fn bind(&self, spec: &WorkspaceMount) -> io::Result<MountIdentity>;
    /// The callback must complete after allocating the exclusive detached clone
    /// but before attachment. Callback failure must leave that clone unattached.
    /// There is deliberately no fallback to unjournaled bind.
    fn bind_journaled(
        &self,
        _spec: &WorkspaceMount,
        _before_attach: &mut dyn FnMut(&MountIdentity) -> io::Result<()>,
    ) -> io::Result<MountIdentity> {
        Err(errno(libc::ENOTSUP))
    }
    /// Only an identity from this backend's own successful attach is a claim.
    /// An observed matching source alone must never authorize removal.
    fn attached_claim(&self, _spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        Ok(None)
    }
    /// Called only after current trusted grant/spec and exclusive pre-attach
    /// durable clone identity matched a fresh physical observation. Recheck in
    /// the backend before adopting; returning success must not perform a mount.
    fn adopt_verified_claim(
        &self,
        _spec: &WorkspaceMount,
        _claim: &MountIdentity,
    ) -> io::Result<()> {
        Err(errno(libc::ENOTSUP))
    }
    /// Recovered identity is not sufficient for admission: effective mount
    /// restrictions must match the current trusted deployment policy.
    fn verify_policy(&self, _spec: &WorkspaceMount, _mount: &MountIdentity) -> io::Result<()> {
        Err(errno(libc::ENOTSUP))
    }
    fn unmount(&self, spec: &WorkspaceMount, mount: &MountIdentity) -> io::Result<()>;
}

struct Record {
    spec: WorkspaceMount,
    status: NativeStatus,
    owned_mount: Option<MountIdentity>,
    retired: HashSet<WorkspaceIdentity>,
    recovery_from: Option<NativeState>,
    persist_failed: bool,
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
            recovery_from: None,
            persist_failed: false,
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
    fn success(&mut self, state: NativeState) {
        self.status.state = state;
        self.status.last_errno = None;
        self.status.last_error = None;
    }
    fn journal(&self) -> JournalRecord {
        let mut retired: Vec<_> = self.retired.iter().cloned().collect();
        retired.sort_by(|a, b| {
            (a.epoch, &a.home_node_id, &a.home_session_id).cmp(&(
                b.epoch,
                &b.home_node_id,
                &b.home_session_id,
            ))
        });
        JournalRecord {
            spec: self.spec.clone(),
            status: self.status.clone(),
            owned_mount: self.owned_mount.clone(),
            retired,
        }
    }
}
type SharedRecord = Arc<Mutex<Record>>;
pub struct NativeMountManager<D: MountBackend> {
    namespace: NamespaceIdentity,
    capacity: usize,
    backend: D,
    records: Mutex<HashMap<String, SharedRecord>>,
    journal: Option<Mutex<MountJournal>>,
}
impl<D: MountBackend> NativeMountManager<D> {
    /// Volatile controller for isolated primitive tests. Production lifecycle
    /// uses with_journal; this constructor cannot qualify restart durability.
    pub fn new(namespace: NamespaceIdentity, capacity: usize, backend: D) -> io::Result<Self> {
        if namespace.inode == 0 || capacity == 0 || capacity > 4096 {
            return Err(errno(libc::EINVAL));
        }
        Ok(Self {
            namespace,
            capacity,
            backend,
            records: Mutex::new(HashMap::new()),
            journal: None,
        })
    }
    pub fn with_journal(
        namespace: NamespaceIdentity,
        capacity: usize,
        backend: D,
        journal: MountJournal,
    ) -> io::Result<Self> {
        let snapshot = journal.load()?;
        if snapshot.namespace != namespace {
            return Err(errno(libc::ESTALE));
        }
        if snapshot.records.len() > capacity {
            return Err(errno(libc::ENOSPC));
        }
        let mut manager = Self::new(namespace, capacity, backend)?;
        let mut records = lock(&manager.records)?;
        for saved in snapshot.records {
            let mut record = Record::new(saved.spec);
            record.recovery_from = Some(saved.status.state);
            record.status = saved.status;
            record.status.state = NativeState::Recovering;
            record.status.observed = None; // A stored observation is not current fact.
            record.owned_mount = saved.owned_mount;
            record.retired = saved.retired.into_iter().collect();
            records.insert(
                record.spec.identity.root_id.clone(),
                Arc::new(Mutex::new(record)),
            );
        }
        drop(records);
        manager.journal = Some(Mutex::new(journal));
        Ok(manager)
    }

    /// Caller supplies current trusted Home authority and prepared descriptors.
    /// Consistency checks here are not a replacement for Meta authorization.
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
            return Err(errno(libc::EINVAL));
        }
        let existing = {
            let mut records = lock(&self.records)?;
            if let Some(old) = records.get(&spec.identity.root_id) {
                old.clone()
            } else {
                if records.len() >= self.capacity {
                    return Err(errno(libc::ENOSPC));
                }
                let mut record = Record::new(spec);
                self.persist(&mut record)?;
                let status = record.status.clone();
                records.insert(
                    status.identity.root_id.clone(),
                    Arc::new(Mutex::new(record)),
                );
                return Ok(status);
            }
        };
        let mut old = lock(&existing)?;
        if old.spec == spec {
            if old.persist_failed {
                return Err(errno(libc::EIO));
            }
            self.journal_healthy(&mut old)?;
            return Ok(old.status.clone());
        }
        self.mutable(&mut old)?;
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
        let mut next = Record::new(spec);
        next.retired = old.retired.clone();
        next.retired.insert(old.spec.identity.clone());
        next.status.operation_seq = old
            .status
            .operation_seq
            .checked_add(1)
            .ok_or_else(|| errno(libc::EOVERFLOW))?;
        self.persist(&mut next)?;
        *old = next;
        Ok(old.status.clone())
    }

    pub fn activate(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        self.mutable(&mut record)?;
        if record.status.desired != NativeDesiredState::Native {
            return Err(errno(libc::EINVAL));
        }
        let actual = match self.backend.inspect(&record.spec) {
            Ok(actual) => actual,
            Err(error) => return self.reject(&mut record, NativeState::Recovering, error),
        };
        record.status.observed = actual.clone();
        if record.status.state == NativeState::NativeActive {
            if actual.is_some() && actual == record.owned_mount {
                return Ok(record.status.clone());
            }
            return self.reject(&mut record, NativeState::Recovering, errno(libc::ESTALE));
        }
        if !matches!(
            record.status.state,
            NativeState::FuseReady | NativeState::FuseOnly
        ) {
            return Err(errno(libc::EINVAL));
        }
        if actual.is_some() {
            return self.reject(&mut record, NativeState::Recovering, errno(libc::EEXIST));
        }
        record.advance()?;
        record.status.state = NativeState::Mounting;
        self.persist(&mut record)?;
        let spec = record.spec.clone();
        let outcome = if self.journal.is_some() {
            self.backend.bind_journaled(&spec, &mut |candidate| {
                if !candidate.matches(&spec) {
                    return Err(errno(libc::ESTALE));
                }
                record.owned_mount = Some(candidate.clone());
                self.persist(&mut record)
            })
        } else {
            self.backend.bind(&spec)
        };
        match outcome {
            Ok(mounted) => {
                record.status.observed = Some(mounted.clone());
                if !mounted.matches(&spec) {
                    return self.reject(&mut record, NativeState::Recovering, errno(libc::ESTALE));
                }
                record.owned_mount = Some(mounted);
                record.success(NativeState::NativeActive);
                self.finish(&mut record)
            }
            Err(error) => {
                if record.persist_failed {
                    return Err(error);
                }
                match self.backend.inspect(&spec) {
                    Ok(None) => {
                        record.owned_mount = None;
                        record.status.observed = None;
                        self.reject(&mut record, NativeState::FuseOnly, error)
                    }
                    Ok(Some(mounted)) => {
                        record.owned_mount = None;
                        record.status.observed = Some(mounted.clone());
                        if mounted.matches(&spec) {
                            match self.backend.attached_claim(&spec) {
                                Ok(Some(claim)) if claim == mounted => {
                                    record.owned_mount = Some(claim)
                                }
                                Ok(_) => {}
                                Err(claim_error) => {
                                    record.fail(NativeState::Recovering, &error);
                                    record.status.last_error = Some(format!(
                                        "{error}; attach claim unavailable: {claim_error}"
                                    ));
                                    self.persist(&mut record)?;
                                    return Err(error);
                                }
                            }
                        }
                        self.reject(&mut record, NativeState::Recovering, error)
                    }
                    Err(observation_error) => {
                        record.fail(NativeState::Recovering, &error);
                        record.status.last_error = Some(format!(
                            "{error}; cannot inspect mount: {observation_error}"
                        ));
                        self.persist(&mut record)?;
                        Err(error)
                    }
                }
            }
        }
    }

    /// Requires current trusted grant plus freshly prepared physical identities.
    /// No mutation/adoption follows the stored journal alone.
    pub fn reconcile(&self, trusted_spec: &WorkspaceMount) -> io::Result<NativeStatus> {
        let shared = self.record(&trusted_spec.identity.root_id)?;
        let mut record = lock(&shared)?;
        if record.spec != *trusted_spec {
            return Err(errno(libc::ESTALE));
        }
        if record.persist_failed {
            return Err(errno(libc::EIO));
        }
        let previous = record.recovery_from.unwrap_or(record.status.state);
        let actual = match self.backend.inspect(trusted_spec) {
            Ok(actual) => actual,
            Err(error) => return self.reject(&mut record, NativeState::Recovering, error),
        };
        record.status.observed = actual.clone();
        match actual {
            Some(mounted) => {
                if record.owned_mount.as_ref() != Some(&mounted) || !mounted.matches(trusted_spec) {
                    return self.reject(&mut record, NativeState::Recovering, errno(libc::ESTALE));
                }
                if let Err(error) = self.backend.adopt_verified_claim(trusted_spec, &mounted) {
                    return self.reject(&mut record, NativeState::Recovering, error);
                }
                // Physical ownership has been established, so a policy error
                // can quiesce/clean this export without ever admitting users.
                record.recovery_from = None;
                if record.status.desired == NativeDesiredState::Native
                    && let Err(error) = self.backend.verify_policy(trusted_spec, &mounted)
                {
                    return self.reject(&mut record, NativeState::Recovering, error);
                }
                let state = if record.status.desired == NativeDesiredState::Native {
                    NativeState::NativeActive
                } else {
                    NativeState::Quiescing
                };
                record.success(state);
            }
            None => {
                if record.owned_mount.is_some()
                    && !matches!(previous, NativeState::Mounting | NativeState::Unmounting)
                {
                    let state = if record.status.desired == NativeDesiredState::Detached {
                        NativeState::Draining
                    } else {
                        NativeState::Recovering
                    };
                    return self.reject(&mut record, state, errno(libc::ESTALE));
                }
                // MOUNTING has no admitted native Agent before its final ACK;
                // UNMOUNTING is issued only after quiesce, with normal umount.
                // Exclusive administration forbids external lazy detach. This
                // confirms export state only, never workspace reclaim/fencing.
                record.owned_mount = None;
                let state = if record.status.desired == NativeDesiredState::Detached {
                    NativeState::Detached
                } else if previous == NativeState::FuseOnly {
                    NativeState::FuseOnly
                } else {
                    NativeState::FuseReady
                };
                record.success(state);
            }
        }
        record.recovery_from = None;
        self.finish(&mut record)
    }

    /// Agent/FUSE/peer drain and authority fencing are independent prerequisites.
    pub fn quiesce(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        self.mutable(&mut record)?;
        if record.status.desired == NativeDesiredState::Detached {
            return Ok(record.status.clone());
        }
        record.advance()?;
        record.status.desired = NativeDesiredState::Detached;
        record.status.state = NativeState::Quiescing;
        self.finish(&mut record)
    }
    /// Normal unmount only. DETACHED confirms this export, not reclamation.
    pub fn detach(&self, id: &WorkspaceIdentity) -> io::Result<NativeStatus> {
        let shared = self.record(&id.root_id)?;
        let mut record = lock(&shared)?;
        record.check(id)?;
        self.mutable(&mut record)?;
        if record.status.state == NativeState::Detached {
            return Ok(record.status.clone());
        }
        if record.status.desired != NativeDesiredState::Detached
            || !matches!(
                record.status.state,
                NativeState::Quiescing | NativeState::Draining
            )
        {
            return Err(errno(libc::EINVAL));
        }
        let actual = match self.backend.inspect(&record.spec) {
            Ok(actual) => actual,
            Err(error) => return self.reject(&mut record, NativeState::Draining, error),
        };
        record.status.observed = actual.clone();
        let Some(actual) = actual else {
            if record.owned_mount.is_some() {
                return self.reject(&mut record, NativeState::Draining, errno(libc::ESTALE));
            }
            record.success(NativeState::Detached);
            return self.finish(&mut record);
        };
        if Some(&actual) != record.owned_mount.as_ref() || !actual.matches(&record.spec) {
            return self.reject(&mut record, NativeState::Draining, errno(libc::ESTALE));
        }
        record.advance()?;
        record.status.state = NativeState::Unmounting;
        self.persist(&mut record)?;
        if let Err(error) = self.backend.unmount(&record.spec, &actual) {
            return self.reject(&mut record, NativeState::Draining, error);
        }
        match self.backend.inspect(&record.spec) {
            Ok(None) => {
                record.owned_mount = None;
                record.status.observed = None;
                record.success(NativeState::Detached);
                self.finish(&mut record)
            }
            Ok(Some(mounted)) => {
                record.status.observed = Some(mounted);
                self.reject(&mut record, NativeState::Recovering, errno(libc::ESTALE))
            }
            Err(error) => self.reject(&mut record, NativeState::Recovering, error),
        }
    }
    /// Explicit private-journal maintenance after all roots are reconciled.
    /// Keep registration and root mutations serialized throughout observation
    /// and removal. Files of uncertain origin/state are retained, not promoted.
    pub fn cleanup_journal_orphans(&self) -> io::Result<super::OrphanMaintenance> {
        let journal = self.journal.as_ref().ok_or_else(|| errno(libc::ENOTSUP))?;
        let records = lock(&self.records)?;
        let shared: Vec<_> = records.values().cloned().collect();
        let mut guarded = shared
            .iter()
            .map(|record| lock(record))
            .collect::<io::Result<Vec<_>>>()?;
        for record in &mut guarded {
            self.mutable(record)?;
            if matches!(
                record.status.state,
                NativeState::Recovering | NativeState::Mounting | NativeState::Unmounting
            ) {
                return Err(errno(libc::ESTALE));
            }
            let actual = self.backend.inspect(&record.spec)?;
            if actual != record.status.observed || actual != record.owned_mount {
                return Err(errno(libc::ESTALE));
            }
        }
        let result = lock(journal)?.cleanup_orphans();
        drop(guarded);
        drop(records);
        result
    }

    pub fn status(&self, root_id: &str) -> io::Result<Option<NativeStatus>> {
        let record = lock(&self.records)?.get(root_id).cloned();
        record.map(|r| Ok(lock(&r)?.status.clone())).transpose()
    }
    fn mutable(&self, record: &mut Record) -> io::Result<()> {
        if record.persist_failed {
            return Err(errno(libc::EIO));
        }
        if record.recovery_from.is_some() {
            return Err(errno(libc::ESTALE));
        }
        self.journal_healthy(record)
    }
    fn journal_healthy(&self, record: &mut Record) -> io::Result<()> {
        if let Some(journal) = &self.journal
            && let Err(error) = lock(journal)?.load()
        {
            record.persist_failed = true;
            record.fail(NativeState::Recovering, &error);
            return Err(error);
        }
        Ok(())
    }
    fn persist(&self, record: &mut Record) -> io::Result<()> {
        if let Some(journal) = &self.journal
            && let Err(error) = lock(journal)?.store(record.journal())
        {
            let operation_error = record.status.last_error.clone();
            record.persist_failed = true;
            record.fail(NativeState::Recovering, &error);
            if let Some(operation_error) = operation_error {
                record.status.last_error =
                    Some(format!("{operation_error}; journal commit failed: {error}"));
            }
            return Err(error);
        }
        Ok(())
    }
    fn finish(&self, record: &mut Record) -> io::Result<NativeStatus> {
        self.persist(record)?;
        Ok(record.status.clone())
    }
    fn reject(
        &self,
        record: &mut Record,
        state: NativeState,
        error: io::Error,
    ) -> io::Result<NativeStatus> {
        record.fail(state, &error);
        self.persist(record)?;
        Err(error)
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
