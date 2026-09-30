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
