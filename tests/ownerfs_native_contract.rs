#![cfg(feature = "ownerfs")]

use afs::node::vfs::ownerfs::native::{
    DirectoryIdentity, MountBackend, MountIdentity, MountInfo, NamespaceIdentity,
    NativeMountManager, NativeState, WorkspaceIdentity, WorkspaceMount,
};
use std::{
    io,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct FakeState {
    mounted: Option<MountIdentity>,
    binds: usize,
    unmounts: usize,
    bind_error: Option<i32>,
    unmount_error: Option<i32>,
    partial_bind: bool,
    foreign_after_bind_error: bool,
}
#[derive(Clone, Default)]
struct FakeBackend(Arc<Mutex<FakeState>>);
impl MountBackend for FakeBackend {
    fn inspect(&self, _: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        Ok(self.0.lock().unwrap().mounted.clone())
    }
    fn bind(&self, spec: &WorkspaceMount) -> io::Result<MountIdentity> {
        let mut state = self.0.lock().unwrap();
        state.binds += 1;
        let mounted = MountIdentity {
            mount_id: 41,
            unique_mount_id: 1041,
            namespace: spec.identity.namespace,
            source: spec.source,
            covered_target: spec.target,
        };
        if state.bind_error.is_none() || state.partial_bind {
            state.mounted = Some(mounted.clone());
        }
        if state.foreign_after_bind_error {
            state.mounted = Some(MountIdentity {
                mount_id: 77,
                unique_mount_id: 1077,
                ..mounted.clone()
            });
        }
        if let Some(errno) = state.bind_error {
            return Err(io::Error::from_raw_os_error(errno));
        }
        Ok(mounted)
    }
    fn attached_claim(&self, _: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        let state = self.0.lock().unwrap();
        Ok(if state.partial_bind && !state.foreign_after_bind_error {
            state.mounted.clone()
        } else {
            None
        })
    }
    fn unmount(&self, _: &WorkspaceMount, _: &MountIdentity) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.unmounts += 1;
        if let Some(errno) = state.unmount_error {
            return Err(io::Error::from_raw_os_error(errno));
        }
        state.mounted = None;
        Ok(())
    }
}
fn namespace() -> NamespaceIdentity {
    NamespaceIdentity {
        device: 4,
        inode: 99,
    }
}
fn spec(epoch: u64) -> WorkspaceMount {
    WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: "root-agent1".into(),
            epoch,
            home_node_id: "a".into(),
            home_session_id: "session-a".into(),
            namespace: namespace(),
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
fn manager() -> (NativeMountManager<FakeBackend>, FakeBackend) {
    let backend = FakeBackend::default();
    (
        NativeMountManager::new(namespace(), 2, backend.clone()).unwrap(),
        backend,
    )
}

#[test]
fn mountinfo_decodes_escaped_paths_and_keeps_stacked_ids() {
    let entries=MountInfo::parse(b"41 9 8:1 /root-agent1-e1 /ownerfs/agent\\040one rw,nosuid shared:2 - ext4 /dev/sdb1 rw\n42 41 8:1 /other /ownerfs/agent\\040one ro - ext4 /dev/sdb1 ro\n").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].mount_id, 41);
    assert_eq!(entries[1].mount_id, 42);
    assert_eq!(entries[0].mount_point.to_str(), Some("/ownerfs/agent one"));
    assert_eq!(entries[0].root.to_str(), Some("/root-agent1-e1"));
    assert_eq!(entries[0].filesystem, "ext4");
    assert!(entries[0].mount_options.iter().any(|s| s == "nosuid"));
}
#[test]
fn mountinfo_preserves_non_utf8_names() {
    use std::os::unix::ffi::OsStrExt;
    let entries = MountInfo::parse(b"41 9 8:1 / /ownerfs/a\\377 rw - ext4 /dev/sdb1 rw\n").unwrap();
    assert_eq!(
        entries[0].mount_point.as_os_str().as_bytes(),
        b"/ownerfs/a\xff"
    );
}
#[test]
fn malformed_mountinfo_is_an_error_not_a_partial_inventory() {
    for bad in [
        b"41 9 8:1 / /ownerfs rw ext4 /dev/sdb1 rw".as_slice(),
        b"0 9 8:1 / /ownerfs rw - ext4 /dev/sdb1 rw",
        b"41 9 8:x / /ownerfs rw - ext4 /dev/sdb1 rw",
        b"41 9 8:1 / /ownerfs\\04 rw - ext4 /dev/sdb1 rw",
        b"41 9 8:1 / /ownerfs\\xyz rw - ext4 /dev/sdb1 rw",
    ] {
        assert_eq!(
            MountInfo::parse(bad).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
#[test]
fn duplicate_activation_does_not_stack_mounts() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.register(spec.clone()).unwrap();
    assert_eq!(
        manager.activate(&spec.identity).unwrap().state,
        NativeState::NativeActive
    );
    manager.activate(&spec.identity).unwrap();
    assert_eq!(backend.0.lock().unwrap().binds, 1);
}
#[test]
fn unregistered_existing_mount_is_never_adopted_or_removed() {
    let (manager, backend) = manager();
    let spec = spec(1);
    backend.0.lock().unwrap().mounted = Some(MountIdentity {
        mount_id: 77,
        unique_mount_id: 1077,
        namespace: namespace(),
        source: spec.source,
        covered_target: spec.target,
    });
    manager.register(spec.clone()).unwrap();
    assert_eq!(
        manager.activate(&spec.identity).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let state = backend.0.lock().unwrap();
    assert_eq!(state.binds, 0);
    assert_eq!(state.unmounts, 0);
    assert_eq!(state.mounted.as_ref().unwrap().mount_id, 77);
}
#[test]
fn wrong_namespace_and_zero_epoch_are_rejected_before_side_effects() {
    let (manager, backend) = manager();
    let mut wrong = spec(1);
    wrong.identity.namespace.inode += 1;
    assert_eq!(
        manager.register(wrong).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        manager.register(spec(0)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(backend.0.lock().unwrap().binds, 0);
}
#[test]
fn failed_bind_confirms_unmounted_fuse_fallback() {
    let (manager, backend) = manager();
    let spec = spec(1);
    backend.0.lock().unwrap().bind_error = Some(libc::EPERM);
    manager.register(spec.clone()).unwrap();
    assert_eq!(
        manager.activate(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::EPERM)
    );
    let status = manager.status(&spec.identity.root_id).unwrap().unwrap();
    assert_eq!(status.state, NativeState::FuseOnly);
    assert_eq!(status.last_errno, Some(libc::EPERM));
}
#[test]
fn partial_bind_failure_is_recovering_not_fuse_only() {
    let (manager, backend) = manager();
    let spec = spec(1);
    {
        let mut state = backend.0.lock().unwrap();
        state.bind_error = Some(libc::EIO);
        state.partial_bind = true;
    }
    manager.register(spec.clone()).unwrap();
    assert!(manager.activate(&spec.identity).is_err());
    assert_eq!(
        manager
            .status(&spec.identity.root_id)
            .unwrap()
            .unwrap()
            .state,
        NativeState::Recovering
    );
    assert_eq!(backend.0.lock().unwrap().unmounts, 0);
}
#[test]
fn stale_epoch_and_session_cannot_touch_active_export() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    let mut stale = spec.identity.clone();
    stale.epoch += 1;
    assert_eq!(
        manager.quiesce(&stale).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    stale = spec.identity.clone();
    stale.home_session_id = "other".into();
    assert_eq!(
        manager.detach(&stale).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(backend.0.lock().unwrap().unmounts, 0);
}
#[test]
fn busy_unmount_preserves_mount_and_rejects_name_reuse() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    manager.quiesce(&spec.identity).unwrap();
    backend.0.lock().unwrap().unmount_error = Some(libc::EBUSY);
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::EBUSY)
    );
    assert_eq!(
        manager
            .status(&spec.identity.root_id)
            .unwrap()
            .unwrap()
            .state,
        NativeState::Draining
    );
    let mut next = spec.clone();
    next.identity.epoch += 1;
    assert_eq!(
        manager.register(next).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(backend.0.lock().unwrap().mounted.is_some());
    backend.0.lock().unwrap().unmount_error = None;
    assert_eq!(
        manager.detach(&spec.identity).unwrap().state,
        NativeState::Detached
    );
}
#[test]
fn active_export_cannot_detach_without_quiesce() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(backend.0.lock().unwrap().unmounts, 0);
}
#[test]
fn changed_mount_id_cannot_be_unmounted_as_ours() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    manager.quiesce(&spec.identity).unwrap();
    backend.0.lock().unwrap().mounted.as_mut().unwrap().mount_id += 1;
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(backend.0.lock().unwrap().unmounts, 0);
}
#[test]
fn detached_epoch_can_be_replaced_but_old_command_stays_stale() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    manager.quiesce(&spec.identity).unwrap();
    manager.detach(&spec.identity).unwrap();
    let mut next = spec.clone();
    next.identity.epoch = 2;
    next.source.inode += 1;
    manager.register(next.clone()).unwrap();
    manager.activate(&next.identity).unwrap();
    assert_eq!(
        manager.activate(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(backend.0.lock().unwrap().binds, 2);
}
#[test]
fn bounded_registry_rejects_exhaustion_without_forgetting_live_roots() {
    let (manager, _) = manager();
    let first = spec(1);
    manager.register(first.clone()).unwrap();
    let mut second = first.clone();
    second.identity.root_id = "second".into();
    manager.register(second).unwrap();
    let mut third = first.clone();
    third.identity.root_id = "third".into();
    assert_eq!(
        manager.register(third).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );
    assert_eq!(
        manager
            .status(&first.identity.root_id)
            .unwrap()
            .unwrap()
            .identity,
        first.identity
    );
}

#[test]
fn matching_foreign_mount_after_bind_failure_is_observation_not_ownership() {
    let (manager, backend) = manager();
    let spec = spec(1);
    {
        let mut state = backend.0.lock().unwrap();
        state.bind_error = Some(libc::EPERM);
        state.foreign_after_bind_error = true;
    }
    manager.register(spec.clone()).unwrap();
    assert_eq!(
        manager.activate(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::EPERM)
    );
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    let state = backend.0.lock().unwrap();
    assert_eq!(state.unmounts, 0);
    assert_eq!(state.mounted.as_ref().unwrap().mount_id, 77);
}

#[test]
fn recycled_mountinfo_id_does_not_make_a_replacement_mount_ours() {
    let (manager, backend) = manager();
    let spec = spec(1);
    manager.register(spec.clone()).unwrap();
    manager.activate(&spec.identity).unwrap();
    backend
        .0
        .lock()
        .unwrap()
        .mounted
        .as_mut()
        .unwrap()
        .unique_mount_id += 1;
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    assert_eq!(backend.0.lock().unwrap().unmounts, 0);
}
