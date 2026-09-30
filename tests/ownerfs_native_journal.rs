#![cfg(feature = "ownerfs")]
use afs::node::vfs::ownerfs::native::{
    DirectoryIdentity, JournalRecord, MountIdentity, MountJournal, NamespaceIdentity,
    NativeDesiredState, NativeState, NativeStatus, WorkspaceIdentity, WorkspaceMount,
};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::PermissionsExt,
};

fn record(epoch: u64) -> JournalRecord {
    let ns = NamespaceIdentity {
        device: 4,
        inode: 99,
    };
    let identity = WorkspaceIdentity {
        root_id: "agent1".into(),
        epoch,
        home_node_id: "a".into(),
        home_session_id: "session-a".into(),
        namespace: ns,
    };
    let spec = WorkspaceMount {
        identity: identity.clone(),
        source: DirectoryIdentity {
            device: 8,
            inode: 501,
        },
        target: DirectoryIdentity {
            device: 19,
            inode: 7,
        },
    };
    JournalRecord {
        spec,
        status: NativeStatus {
            identity,
            desired: NativeDesiredState::Native,
            state: NativeState::Mounting,
            operation_seq: 1,
            observed: None,
            last_errno: None,
            last_error: None,
        },
        owned_mount: None,
    }
}
fn open(dir: &tempfile::TempDir) -> io::Result<MountJournal> {
    MountJournal::open(
        File::open(dir.path())?,
        "00000000-0000-0000-0000-000000000001",
        NamespaceIdentity {
            device: 4,
            inode: 99,
        },
        8,
    )
}

#[test]
fn journal_preserves_pre_syscall_intent_and_post_syscall_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir).unwrap();
    let mut entry = record(1);
    journal.store(entry.clone()).unwrap();
    entry.status.state = NativeState::NativeActive;
    entry.owned_mount = Some(MountIdentity {
        mount_id: 41,
        unique_mount_id: 1041,
        namespace: entry.spec.identity.namespace,
        source: entry.spec.source,
        covered_target: entry.spec.target,
    });
    entry.status.observed = entry.owned_mount.clone();
    journal.store(entry.clone()).unwrap();
    drop(journal);
    let journal = open(&dir).unwrap();
    let snapshot = journal.load().unwrap();
    assert_eq!(snapshot.records, vec![entry]);
    assert_eq!(snapshot.boot_id, "00000000-0000-0000-0000-000000000001");
}

#[test]
fn journal_serializes_exclusive_manager_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let journal = open(&dir).unwrap();
    assert_eq!(
        open(&dir).err().unwrap().raw_os_error(),
        Some(libc::EWOULDBLOCK)
    );
    drop(journal);
    open(&dir).unwrap();
}

#[test]
fn truncated_or_unknown_schema_is_not_silently_discarded() {
    let dir = tempfile::tempdir().unwrap();
    for bytes in [b"{".as_slice(), b"{\"schema_version\":999}".as_slice()] {
        fs::write(dir.path().join("state.json"), bytes).unwrap();
        fs::set_permissions(
            dir.path().join("state.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(open(&dir).err().unwrap().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(dir.path().join("state.json")).unwrap(), bytes);
    }
}

#[test]
fn journal_does_not_follow_lock_or_state_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), b"must survive").unwrap();
    symlink(outside.path(), dir.path().join("state.json")).unwrap();
    assert!(open(&dir).is_err());
    fs::remove_file(dir.path().join("state.json")).unwrap();
    fs::remove_file(dir.path().join("manager.lock")).unwrap();
    symlink(outside.path(), dir.path().join("manager.lock")).unwrap();
    assert!(open(&dir).is_err());
    assert_eq!(fs::read(outside.path()).unwrap(), b"must survive");
}

#[test]
fn stale_operation_and_unmatched_claim_cannot_replace_durable_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir).unwrap();
    let entry = record(1);
    journal.store(entry.clone()).unwrap();
    let mut stale = entry.clone();
    stale.status.operation_seq = 0;
    assert_eq!(
        journal.store(stale).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    let mut foreign = entry.clone();
    foreign.owned_mount = Some(MountIdentity {
        mount_id: 77,
        unique_mount_id: 1077,
        namespace: foreign.spec.identity.namespace,
        source: DirectoryIdentity {
            device: 8,
            inode: 999,
        },
        covered_target: foreign.spec.target,
    });
    assert!(journal.store(foreign).is_err());
    assert_eq!(journal.load().unwrap().records, vec![entry]);
}

#[test]
fn current_namespace_mismatch_preserves_old_recovery_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir).unwrap();
    journal.store(record(1)).unwrap();
    drop(journal);
    let old = fs::read(dir.path().join("state.json")).unwrap();
    let result = MountJournal::open(
        File::open(dir.path()).unwrap(),
        "00000000-0000-0000-0000-000000000001",
        NamespaceIdentity {
            device: 4,
            inode: 100,
        },
        8,
    );
    assert_eq!(result.err().unwrap().raw_os_error(), Some(libc::ESTALE));
    assert_eq!(fs::read(dir.path().join("state.json")).unwrap(), old);
}

#[test]
fn mount_intent_cannot_forget_prior_live_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir).unwrap();
    journal.store(record(1)).unwrap();
    assert_eq!(
        journal.store(record(2)).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(journal.load().unwrap().records, vec![record(1)]);
}

#[test]
fn failed_directory_sync_requires_reopen_instead_of_serving_stale_memory() {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    // A live O_PATH descriptor admits openat/renameat but cannot fsync. This
    // deterministically exercises an error after the atomic rename succeeded.
    let path_fd = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(dir.path())
        .unwrap();
    let mut journal = MountJournal::open(
        path_fd,
        "00000000-0000-0000-0000-000000000001",
        NamespaceIdentity {
            device: 4,
            inode: 99,
        },
        8,
    )
    .unwrap();
    assert_eq!(
        journal.store(record(1)).unwrap_err().raw_os_error(),
        Some(libc::EBADF)
    );
    assert_eq!(journal.load().unwrap_err().raw_os_error(), Some(libc::EIO));
    assert_eq!(
        journal.store(record(1)).unwrap_err().raw_os_error(),
        Some(libc::EIO)
    );
    drop(journal);
    assert_eq!(open(&dir).unwrap().load().unwrap().records, vec![record(1)]);
}
