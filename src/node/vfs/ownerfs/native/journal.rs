//! Durable export intent. A persisted mount is a claim requiring live verification.
use super::{
    MountIdentity, NamespaceIdentity, NativeDesiredState, NativeState, NativeStatus,
    WorkspaceIdentity, WorkspaceMount,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::MetadataExt,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub spec: WorkspaceMount,
    pub status: NativeStatus,
    /// Exclusive clone intent saved before attachment; never proof that it is mounted.
    pub owned_mount: Option<MountIdentity>,
    pub retired: Vec<WorkspaceIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalSnapshot {
    pub schema_version: u32,
    pub boot_id: String,
    pub namespace: NamespaceIdentity,
    pub records: Vec<JournalRecord>,
}

pub struct MountJournal {
    directory: File,
    _exclusive_lock: File,
    snapshot: JournalSnapshot,
    capacity: usize,
    uncertain: bool,
}

impl MountJournal {
    /// The caller supplies a trusted manager-private directory descriptor.
    /// Root authority and current physical descriptors must still be validated
    /// by recovery; this API cannot turn stored claims into mount ownership.
    pub fn open(
        directory: File,
        boot_id: &str,
        namespace: NamespaceIdentity,
        capacity: usize,
    ) -> io::Result<Self> {
        let metadata = directory.metadata()?;
        if !metadata.is_dir()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o022 != 0
            || capacity == 0
            || capacity > 4096
            || namespace.inode == 0
            || !valid_boot_id(boot_id)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid private journal directory/identity/bounds",
            ));
        }
        let exclusive_lock = openat(&directory, "manager.lock", libc::O_RDWR | libc::O_CREAT)?;
        check_regular(&exclusive_lock)?;
        exclusive_lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => errno(libc::EWOULDBLOCK),
            std::fs::TryLockError::Error(error) => error,
        })?;
        let snapshot = match openat(&directory, "state.json", libc::O_RDONLY) {
            Ok(file) => {
                check_regular(&file)?;
                if file.metadata()?.len() > MAX_BYTES as u64 {
                    return Err(invalid("journal too large"));
                }
                let mut bytes = Vec::new();
                file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                if bytes.len() > MAX_BYTES {
                    return Err(invalid("journal too large"));
                }
                let snapshot: JournalSnapshot =
                    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
                validate_snapshot(&snapshot, capacity)?;
                if snapshot.boot_id != boot_id || snapshot.namespace != namespace {
                    return Err(errno(libc::ESTALE));
                }
                snapshot
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => JournalSnapshot {
                schema_version: 2,
                boot_id: boot_id.into(),
                namespace,
                records: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        Ok(Self {
            directory,
            _exclusive_lock: exclusive_lock,
            snapshot,
            capacity,
            uncertain: false,
        })
    }

    pub fn load(&self) -> io::Result<JournalSnapshot> {
        if self.uncertain {
            return Err(errno(libc::EIO));
        }
        Ok(self.snapshot.clone())
    }

    /// Fsync the new file before rename, then fsync its directory. On any
    /// failure callers must stop mutation and reopen/reconcile; an error does
    /// not prove that the new intent is absent from disk.
    pub fn store(&mut self, record: JournalRecord) -> io::Result<()> {
        if self.uncertain {
            return Err(errno(libc::EIO));
        }
        validate_record(&record, self.snapshot.namespace)?;
        let mut next = self.snapshot.clone();
        if let Some(index) = next
            .records
            .iter()
            .position(|r| r.spec.identity.root_id == record.spec.identity.root_id)
        {
            let old = &next.records[index];
            if !old.retired.iter().all(|id| record.retired.contains(id))
                || (old.spec.identity != record.spec.identity
                    && !record.retired.contains(&old.spec.identity))
            {
                return Err(errno(libc::ESTALE));
            }
            if old.spec.identity == record.spec.identity {
                if old.spec != record.spec || record.status.operation_seq < old.status.operation_seq
                {
                    return Err(errno(libc::ESTALE));
                }
            } else if old.status.state != NativeState::Detached
                || old.status.desired != NativeDesiredState::Detached
                || old.owned_mount.is_some()
                || old.status.observed.is_some()
                || record.spec.identity.epoch < old.spec.identity.epoch
                || record.status.operation_seq <= old.status.operation_seq
            {
                return Err(errno(libc::ESTALE));
            }
            next.records[index] = record;
        } else {
            if next.records.len() >= self.capacity {
                return Err(errno(libc::ENOSPC));
            }
            next.records.push(record);
            next.records
                .sort_by(|a, b| a.spec.identity.root_id.cmp(&b.spec.identity.root_id));
        }
        let bytes = serde_json::to_vec(&next).map_err(io::Error::other)?;
        if bytes.len() > MAX_BYTES {
            return Err(errno(libc::ENOSPC));
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let temp_name = format!(".state-{}-{stamp}", std::process::id());
        let mut file = openat(
            &self.directory,
            &temp_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        // Recognizable orphan files are preserved for explicit recovery rather
        // than blindly deleted on errors or future startups.
        self.uncertain = true;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let from = CString::new(temp_name).map_err(io::Error::other)?;
        let to = c"state.json";
        renameat(&self.directory, &from, to)?;
        self.directory.sync_all()?;
        self.snapshot = next;
        self.uncertain = false;
        Ok(())
    }
}

fn validate_snapshot(snapshot: &JournalSnapshot, capacity: usize) -> io::Result<()> {
    if snapshot.schema_version != 2
        || !valid_boot_id(&snapshot.boot_id)
        || snapshot.namespace.inode == 0
        || snapshot.records.len() > capacity
    {
        return Err(invalid("invalid journal version/identity/bounds"));
    }
    let mut roots = std::collections::HashSet::new();
    for record in &snapshot.records {
        validate_record(record, snapshot.namespace)?;
        if !roots.insert(&record.spec.identity.root_id) {
            return Err(invalid("duplicate journal root"));
        }
    }
    Ok(())
}
fn validate_record(record: &JournalRecord, namespace: NamespaceIdentity) -> io::Result<()> {
    let identity = &record.spec.identity;
    let mut retired = std::collections::HashSet::new();
    if record.retired.len() > 64
        || record.retired.iter().any(|old| {
            old == identity
                || old.root_id != identity.root_id
                || old.namespace != namespace
                || old.epoch == 0
                || old.epoch > identity.epoch
                || old.home_node_id.is_empty()
                || old.home_node_id.len() > 1024
                || old.home_session_id.is_empty()
                || old.home_session_id.len() > 1024
                || !retired.insert(old)
        })
    {
        return Err(invalid("invalid retired Home session history"));
    }
    if record.status.identity != *identity
        || identity.namespace != namespace
        || identity.epoch == 0
        || record.spec.source.inode == 0
        || record.spec.target.inode == 0
        || [
            &identity.root_id,
            &identity.home_node_id,
            &identity.home_session_id,
        ]
        .iter()
        .any(|s| s.is_empty() || s.len() > 1024)
        || record
            .status
            .last_error
            .as_ref()
            .is_some_and(|s| s.len() > 8192)
        || record
            .owned_mount
            .as_ref()
            .is_some_and(|m| !m.matches(&record.spec))
        || (record.status.state == NativeState::NativeActive
            && (record.owned_mount.is_none() || record.status.observed != record.owned_mount))
        || (record.status.state == NativeState::Detached
            && (record.status.desired != NativeDesiredState::Detached
                || record.owned_mount.is_some()
                || record.status.observed.is_some()))
    {
        return Err(invalid("invalid journal record/ownership claim"));
    }
    Ok(())
}
fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn check_regular(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid(
            "journal file must be private, regular and singly linked",
        ));
    }
    Ok(())
}
#[allow(unsafe_code)]
fn openat(directory: &File, name: &str, flags: i32) -> io::Result<File> {
    let name = CString::new(name).map_err(io::Error::other)?;
    // SAFETY: directory fd is live; the NUL-terminated name is not retained.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat transfers a fresh owned fd to File exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn errno(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[allow(unsafe_code)]
fn effective_uid() -> libc::uid_t {
    // SAFETY: geteuid has no pointer arguments or memory safety preconditions.
    unsafe { libc::geteuid() }
}
#[allow(unsafe_code)]
fn renameat(directory: &File, from: &std::ffi::CStr, to: &std::ffi::CStr) -> io::Result<()> {
    // SAFETY: the directory fd and NUL-terminated names remain live; no pointers are retained.
    let result = unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
