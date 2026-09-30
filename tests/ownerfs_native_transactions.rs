#![cfg(feature = "ownerfs")]
use afs::node::vfs::ownerfs::native::{
    DirectoryIdentity, JournalRecord, JournalSnapshot, MountBackend, MountIdentity, MountJournal,
    NamespaceIdentity, NativeMountManager, NativeState, WorkspaceIdentity, WorkspaceMount,
};
use std::{
    fs::{self, File},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct KernelState {
    mounted: Option<MountIdentity>,
    binds: usize,
    unmounts: usize,
    crash_before_attach: bool,
    crash_after_attach: bool,
    crash_after_unmount: bool,
}
#[derive(Clone)]
struct Spy {
    kernel: Arc<Mutex<KernelState>>,
    journal_path: PathBuf,
}
impl Spy {
    fn journal_record(&self) -> JournalRecord {
        let snapshot: JournalSnapshot =
            serde_json::from_slice(&fs::read(&self.journal_path).unwrap()).unwrap();
        assert_eq!(snapshot.records.len(), 1);
        snapshot.records[0].clone()
    }
}
impl MountBackend for Spy {
    fn inspect(&self, _: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        Ok(self.kernel.lock().unwrap().mounted.clone())
    }
    fn bind(&self, _: &WorkspaceMount) -> io::Result<MountIdentity> {
        panic!("journaled controller used unjournaled bind")
    }
    fn bind_journaled(
        &self,
        spec: &WorkspaceMount,
        before_attach: &mut dyn FnMut(&MountIdentity) -> io::Result<()>,
    ) -> io::Result<MountIdentity> {
        assert_eq!(self.journal_record().status.state, NativeState::Mounting);
        let candidate = MountIdentity {
            mount_id: 41,
            unique_mount_id: 1041,
            namespace: spec.identity.namespace,
            source: spec.source,
            covered_target: spec.target,
        };
        before_attach(&candidate)?;
        let intent = self.journal_record();
        assert_eq!(intent.status.state, NativeState::Mounting);
        assert_eq!(intent.owned_mount, Some(candidate.clone()));
        let mut kernel = self.kernel.lock().unwrap();
        kernel.binds += 1;
        let before = kernel.crash_before_attach;
        let after = kernel.crash_after_attach;
        if !before {
            kernel.mounted = Some(candidate.clone());
        }
        drop(kernel);
        if before {
            panic!("crash with durable clone intent, before attach");
        }
        if after {
            panic!("crash after attach, before final ACK");
        }
        Ok(candidate)
    }
    fn adopt_verified_claim(&self, spec: &WorkspaceMount, claim: &MountIdentity) -> io::Result<()> {
        if self.inspect(spec)?.as_ref() != Some(claim) {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
        Ok(())
    }
    fn verify_policy(&self, _: &WorkspaceMount, _: &MountIdentity) -> io::Result<()> {
        Ok(())
    }
    fn unmount(&self, _: &WorkspaceMount, mount: &MountIdentity) -> io::Result<()> {
        let intent = self.journal_record();
        assert_eq!(intent.status.state, NativeState::Unmounting);
        assert_eq!(intent.owned_mount.as_ref(), Some(mount));
        let mut kernel = self.kernel.lock().unwrap();
        kernel.unmounts += 1;
        kernel.mounted = None;
        let crash = kernel.crash_after_unmount;
        drop(kernel);
        if crash {
            panic!("crash after normal unmount, before final ACK");
        }
        Ok(())
    }
}
fn spec(session: &str) -> WorkspaceMount {
    WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: "agent1".into(),
            epoch: 1,
            home_node_id: "a".into(),
            home_session_id: session.into(),
            namespace: NamespaceIdentity {
                device: 4,
                inode: 99,
            },
        },
        source: DirectoryIdentity {
            device: 8,
            inode: 501,
        },
        target: DirectoryIdentity {
            device: 19,
            inode: 7,
        },
    }
}
fn new_manager(path: &Path, backend: Spy) -> NativeMountManager<Spy> {
    let ns = spec("a").identity.namespace;
    let journal = MountJournal::open(
        File::open(path).unwrap(),
        "00000000-0000-0000-0000-000000000001",
        ns,
        8,
    )
    .unwrap();
    NativeMountManager::with_journal(ns, 8, backend, journal).unwrap()
}
fn fixture() -> (tempfile::TempDir, PathBuf, Spy) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal");
    fs::create_dir(&path).unwrap();
    let spy = Spy {
        kernel: Arc::default(),
        journal_path: path.join("state.json"),
    };
    (dir, path, spy)
}

#[test]
fn kernel_attach_and_unmount_are_bracketed_by_durable_intent_and_ack() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    let active = manager.activate(&workspace.identity).unwrap();
    assert_eq!(active.state, NativeState::NativeActive);
    assert_eq!(spy.journal_record().status, active);
    manager.quiesce(&workspace.identity).unwrap();
    let detached = manager.detach(&workspace.identity).unwrap();
    assert_eq!(detached.state, NativeState::Detached);
    assert_eq!(spy.journal_record().status, detached);
    assert_eq!(spy.journal_record().owned_mount, None);
}

#[test]
fn crash_before_or_after_attach_reconciles_actual_kernel_without_rebinding() {
    for before in [true, false] {
        let (_dir, path, spy) = fixture();
        let workspace = spec("session-a");
        {
            let mut kernel = spy.kernel.lock().unwrap();
            kernel.crash_before_attach = before;
            kernel.crash_after_attach = !before;
        }
        let manager = new_manager(&path, spy.clone());
        manager.register(workspace.clone()).unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| manager.activate(&workspace.identity))).is_err());
        assert_eq!(spy.journal_record().status.state, NativeState::Mounting);
        drop(manager);
        let restored = new_manager(&path, spy.clone());
        assert_eq!(
            restored.status("agent1").unwrap().unwrap().state,
            NativeState::Recovering
        );
        assert!(restored.activate(&workspace.identity).is_err());
        let status = restored.reconcile(&workspace).unwrap();
        assert_eq!(
            status.state,
            if before {
                NativeState::FuseReady
            } else {
                NativeState::NativeActive
            }
        );
        assert_eq!(spy.kernel.lock().unwrap().binds, 1);
    }
}

#[test]
fn crash_after_normal_unmount_does_not_recreate_deleted_export() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    manager.activate(&workspace.identity).unwrap();
    manager.quiesce(&workspace.identity).unwrap();
    spy.kernel.lock().unwrap().crash_after_unmount = true;
    assert!(catch_unwind(AssertUnwindSafe(|| manager.detach(&workspace.identity))).is_err());
    assert_eq!(spy.journal_record().status.state, NativeState::Unmounting);
    drop(manager);
    let restored = new_manager(&path, spy.clone());
    assert_eq!(
        restored.reconcile(&workspace).unwrap().state,
        NativeState::Detached
    );
    assert_eq!(spy.kernel.lock().unwrap().binds, 1);
    assert_eq!(spy.kernel.lock().unwrap().unmounts, 1);
}

#[test]
fn restart_requires_current_identity_and_refuses_matching_foreign_mount() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    manager.activate(&workspace.identity).unwrap();
    drop(manager);
    let restored = new_manager(&path, spy.clone());
    assert_eq!(
        restored
            .reconcile(&spec("session-b"))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESTALE)
    );
    spy.kernel
        .lock()
        .unwrap()
        .mounted
        .as_mut()
        .unwrap()
        .unique_mount_id += 1;
    assert_eq!(
        restored.reconcile(&workspace).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(spy.kernel.lock().unwrap().unmounts, 0);
}

#[test]
fn deleted_journal_prevents_any_attach_after_registration() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    fs::remove_dir_all(&path).unwrap();
    assert!(manager.activate(&workspace.identity).is_err());
    assert_eq!(spy.kernel.lock().unwrap().binds, 0);
    assert_eq!(
        manager.status("agent1").unwrap().unwrap().state,
        NativeState::Recovering
    );
    assert!(manager.activate(&workspace.identity).is_err());
}

#[test]
fn same_epoch_retired_home_session_survives_manager_restart() {
    let (_dir, path, spy) = fixture();
    let old = spec("session-a");
    let new = spec("session-b");
    let manager = new_manager(&path, spy.clone());
    manager.register(old.clone()).unwrap();
    manager.quiesce(&old.identity).unwrap();
    manager.detach(&old.identity).unwrap();
    manager.register(new.clone()).unwrap();
    drop(manager);
    let restored = new_manager(&path, spy.clone());
    restored.reconcile(&new).unwrap();
    restored.quiesce(&new.identity).unwrap();
    restored.detach(&new.identity).unwrap();
    assert_eq!(
        restored.register(old).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
}

fn private_orphan(path: &Path, name: &str, data: &[u8]) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path.join(name))
        .unwrap();
    file.write_all(data).unwrap();
}

#[test]
fn orphan_cleanup_requires_reconciliation_and_preserves_unknown_files() {
    use std::os::unix::fs::symlink;
    let (dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    let bytes = fs::read(path.join("state.json")).unwrap();
    private_orphan(&path, ".state-123-100", &bytes);
    private_orphan(&path, ".state-123-101", b"unrecognized partial data");
    private_orphan(&path, "operator-note", b"keep this");
    let outside = dir.path().join("outside");
    fs::write(&outside, b"untouched").unwrap();
    symlink(&outside, path.join(".state-123-102")).unwrap();
    drop(manager);
    let restored = new_manager(&path, spy.clone());
    assert!(restored.cleanup_journal_orphans().is_err());
    assert!(path.join(".state-123-100").exists());
    restored.reconcile(&workspace).unwrap();
    let report = restored.cleanup_journal_orphans().unwrap();
    assert_eq!(report.removed, vec![".state-123-100"]);
    for name in [".state-123-101", ".state-123-102", "operator-note"] {
        assert!(
            report.retained.contains(&name.to_owned()),
            "missing retained {name}"
        );
    }
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    assert_eq!(fs::read(path.join("state.json")).unwrap(), bytes);
    assert_eq!(spy.kernel.lock().unwrap().binds, 0);
    assert!(
        restored
            .cleanup_journal_orphans()
            .unwrap()
            .removed
            .is_empty()
    );
}

#[test]
fn orphan_cleanup_preserves_foreign_future_and_hardlinked_snapshots() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    let original = fs::read(path.join("state.json")).unwrap();
    let mut snapshot: JournalSnapshot = serde_json::from_slice(&original).unwrap();
    snapshot.boot_id = "00000000-0000-0000-0000-000000000002".into();
    private_orphan(
        &path,
        ".state-123-200",
        &serde_json::to_vec(&snapshot).unwrap(),
    );
    snapshot.boot_id = "00000000-0000-0000-0000-000000000001".into();
    snapshot.records[0].status.operation_seq += 1;
    private_orphan(
        &path,
        ".state-123-201",
        &serde_json::to_vec(&snapshot).unwrap(),
    );
    snapshot.records[0].status.operation_seq = 0;
    snapshot.records[0].spec.identity.home_session_id = "unknown-session".into();
    snapshot.records[0].status.identity = snapshot.records[0].spec.identity.clone();
    private_orphan(
        &path,
        ".state-123-202",
        &serde_json::to_vec(&snapshot).unwrap(),
    );
    private_orphan(&path, ".state-123-203", &original);
    fs::hard_link(path.join(".state-123-203"), path.join("outside-hardlink")).unwrap();
    let report = manager.cleanup_journal_orphans().unwrap();
    assert!(report.removed.is_empty());
    for name in [
        ".state-123-200",
        ".state-123-201",
        ".state-123-202",
        ".state-123-203",
    ] {
        assert!(path.join(name).is_file());
        assert!(report.retained.contains(&name.to_owned()));
    }
    assert_eq!(fs::read(path.join("state.json")).unwrap(), original);
}

#[test]
fn orphan_cleanup_never_precedes_fresh_physical_observation() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    let original = fs::read(path.join("state.json")).unwrap();
    private_orphan(&path, ".state-123-300", &original);
    spy.kernel.lock().unwrap().mounted = Some(MountIdentity {
        mount_id: 45,
        unique_mount_id: 9000,
        namespace: workspace.identity.namespace,
        source: workspace.source,
        covered_target: workspace.target,
    });
    assert_eq!(
        manager
            .cleanup_journal_orphans()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESTALE)
    );
    assert!(path.join(".state-123-300").exists());
    assert_eq!(fs::read(path.join("state.json")).unwrap(), original);
}

#[test]
fn orphan_directory_scan_is_bounded_before_any_removal() {
    let (_dir, path, spy) = fixture();
    let manager = new_manager(&path, spy.clone());
    manager.register(spec("session-a")).unwrap();
    let original = fs::read(path.join("state.json")).unwrap();
    private_orphan(&path, ".state-123-400", &original);
    for index in 0..256 {
        private_orphan(&path, &format!("note-{index}"), b"preserve");
    }
    assert_eq!(
        manager
            .cleanup_journal_orphans()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ENOSPC)
    );
    assert!(path.join(".state-123-400").exists());
    assert_eq!(fs::read(path.join("state.json")).unwrap(), original);
}

#[test]
fn orphan_unlink_directory_sync_failure_requires_reopen() {
    use std::os::unix::fs::OpenOptionsExt;
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let namespace = workspace.identity.namespace;
    let journal = MountJournal::open(
        File::open(&path).unwrap(),
        "00000000-0000-0000-0000-000000000001",
        namespace,
        8,
    )
    .unwrap();
    private_orphan(
        &path,
        ".state-123-500",
        &serde_json::to_vec(&journal.load().unwrap()).unwrap(),
    );
    drop(journal);
    // O_PATH supports fd-relative lookup/unlink but cannot be fsynced. There
    // are no roots here, so no unverified recovered record may be admitted.
    let descriptor = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(&path)
        .unwrap();
    let journal = MountJournal::open(
        descriptor,
        "00000000-0000-0000-0000-000000000001",
        namespace,
        8,
    )
    .unwrap();
    let manager = NativeMountManager::with_journal(namespace, 8, spy.clone(), journal).unwrap();
    assert_eq!(
        manager
            .cleanup_journal_orphans()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EBADF)
    );
    assert!(!path.join(".state-123-500").exists());
    assert_eq!(
        manager
            .cleanup_journal_orphans()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EIO)
    );
    assert_eq!(
        manager
            .register(workspace.clone())
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EIO)
    );
    assert_eq!(spy.kernel.lock().unwrap().binds, 0);
    drop(manager);
    let reopened = new_manager(&path, spy);
    reopened.register(workspace).unwrap();
    assert!(
        reopened
            .cleanup_journal_orphans()
            .unwrap()
            .removed
            .is_empty()
    );
}

#[test]
fn duplicate_registration_rejects_global_uncertain_journal() {
    let (_dir, path, spy) = fixture();
    let workspace = spec("session-a");
    let manager = new_manager(&path, spy.clone());
    manager.register(workspace.clone()).unwrap();
    // Fail rename after a temp has been written/fsynced, independently of UID.
    // The failure belongs to a different root: the old record itself has not
    // seen a persistence error, but its duplicate ACK must check journal health.
    fs::rename(path.join("state.json"), path.join("saved-state")).unwrap();
    fs::create_dir(path.join("state.json")).unwrap();
    let mut other = workspace.clone();
    other.identity.root_id = "agent2".into();
    assert!(manager.register(other).is_err());
    assert_eq!(
        manager
            .register(workspace.clone())
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EIO)
    );
    assert_eq!(spy.kernel.lock().unwrap().binds, 0);
    drop(manager);
    fs::remove_dir(path.join("state.json")).unwrap();
    fs::rename(path.join("saved-state"), path.join("state.json")).unwrap();
    let restored = new_manager(&path, spy);
    restored.reconcile(&workspace).unwrap();
    assert_eq!(
        restored.register(workspace).unwrap().state,
        NativeState::FuseReady
    );
}
