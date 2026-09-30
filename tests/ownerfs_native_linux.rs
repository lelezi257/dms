#![cfg(feature = "ownerfs")]
use afs::node::vfs::ownerfs::native::LinuxMountBackend;
use afs::node::vfs::ownerfs::native::MountPolicy;
use afs::node::vfs::ownerfs::native::{
    DirectoryIdentity, MountBackend, NativeMountManager, NativeState, WorkspaceIdentity,
    WorkspaceMount,
};
use std::{
    fs::{self, File},
    os::unix::fs::MetadataExt,
};

fn dir_id(path: &std::path::Path) -> DirectoryIdentity {
    let stat = fs::metadata(path).unwrap();
    DirectoryIdentity {
        device: stat.dev(),
        inode: stat.ino(),
    }
}
fn setup(dir: &tempfile::TempDir) -> (LinuxMountBackend, WorkspaceMount) {
    fs::create_dir(dir.path().join("source")).unwrap();
    fs::create_dir(dir.path().join("parent")).unwrap();
    fs::create_dir(dir.path().join("parent/agent1")).unwrap();
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    let spec = WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: "root1".into(),
            epoch: 1,
            home_node_id: "a".into(),
            home_session_id: "session-a".into(),
            namespace,
        },
        source: dir_id(&dir.path().join("source")),
        target: dir_id(&dir.path().join("parent/agent1")),
    };
    backend
        .prepare(
            spec.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    (backend, spec)
}

#[test]
#[ignore = "requires Linux >= 6.8 STATX_MNT_ID_UNIQUE; run in VM"]
fn prepared_backend_rejects_replacement_and_symlink_targets() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let (backend, spec) = setup(&dir);
    assert_eq!(backend.inspect(&spec).unwrap(), None);
    fs::rename(
        dir.path().join("parent/agent1"),
        dir.path().join("parent/old"),
    )
    .unwrap();
    fs::create_dir(dir.path().join("parent/agent1")).unwrap();
    assert_eq!(
        backend.inspect(&spec).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    fs::remove_dir(dir.path().join("parent/agent1")).unwrap();
    symlink(dir.path().join("source"), dir.path().join("parent/agent1")).unwrap();
    assert!(backend.inspect(&spec).is_err());
}

#[test]
#[ignore = "requires Linux >= 6.8 STATX_MNT_ID_UNIQUE; run in VM"]
fn backend_pins_source_inode_across_source_rename() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, spec) = setup(&dir);
    fs::rename(
        dir.path().join("source"),
        dir.path().join("original-source"),
    )
    .unwrap();
    fs::create_dir(dir.path().join("source")).unwrap();
    assert_eq!(backend.inspect(&spec).unwrap(), None);
}

#[test]
#[ignore = "requires Linux >= 6.8 STATX_MNT_ID_UNIQUE; run in VM"]
fn backend_rejects_unprepared_or_rebound_spec() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, spec) = setup(&dir);
    let mut other = spec;
    other.source.inode += 1;
    assert_eq!(
        backend.inspect(&other).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    other.identity.root_id = "unknown".into();
    assert_eq!(
        backend.inspect(&other).unwrap_err().raw_os_error(),
        Some(libc::ENOENT)
    );
}

/// Execute only inside an explicitly isolated private mount namespace in the
/// Linux VM, with the temp directory located on its identified ext4 data disk.
#[test]
#[ignore = "requires CAP_SYS_ADMIN, private mount namespace, and VM ext4 TMPDIR"]
fn privileged_native_lifecycle() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let (backend, spec) = setup(&dir);
    fs::write(dir.path().join("source/file"), b"same backing").unwrap();
    let before = File::open(dir.path().join("parent/agent1")).unwrap();
    let manager = NativeMountManager::new(spec.identity.namespace, 8, backend).unwrap();
    manager.register(spec.clone()).unwrap();
    let active = manager.activate(&spec.identity).unwrap();
    assert_eq!(active.state, NativeState::NativeActive);
    println!("verified_export={:?}", active.observed);
    let info = afs::node::vfs::ownerfs::native::MountInfo::parse(
        &fs::read("/proc/thread-self/mountinfo").unwrap(),
    )
    .unwrap();
    let ours = info
        .iter()
        .find(|m| m.mount_id == active.observed.as_ref().unwrap().mount_id)
        .unwrap();
    assert!(ours.mount_options.iter().any(|v| v == "nosuid"));
    assert!(ours.mount_options.iter().any(|v| v == "nodev"));
    assert_eq!(dir_id(&dir.path().join("parent/agent1")), spec.source);
    assert_eq!(before.metadata().unwrap().ino(), spec.target.inode);
    assert_eq!(
        fs::read(dir.path().join("parent/agent1/file")).unwrap(),
        b"same backing"
    );
    fs::write(dir.path().join("parent/agent1/file"), b"native update").unwrap();
    assert_eq!(
        fs::read(dir.path().join("source/file")).unwrap(),
        b"native update"
    );
    assert_eq!(
        manager.activate(&spec.identity).unwrap().observed,
        active.observed
    );
    let live = File::open(dir.path().join("parent/agent1/file")).unwrap();
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap_err().raw_os_error(),
        Some(libc::EBUSY)
    );
    assert_eq!(
        manager.status("root1").unwrap().unwrap().state,
        NativeState::Draining
    );
    drop(live);
    assert_eq!(
        manager.detach(&spec.identity).unwrap().state,
        NativeState::Detached
    );
    assert_eq!(dir_id(&dir.path().join("parent/agent1")), spec.target);
    assert_eq!(
        fs::read(dir.path().join("source/file")).unwrap(),
        b"native update"
    );
}

#[test]
#[ignore = "requires CAP_SYS_ADMIN, private mount namespace, Linux >= 6.8, VM ext4"]
fn privileged_readonly_noexec_policy_and_foreign_mount() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let (first, spec) = setup(&dir);
    // A separately prepared manager must not adopt or remove this manager's export.
    let second = LinuxMountBackend::new(spec.identity.namespace, 8).unwrap();
    second
        .prepare(
            spec.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let mounted = first.bind(&spec).unwrap();
    assert_eq!(
        second.bind(&spec).unwrap_err().raw_os_error(),
        Some(libc::EEXIST)
    );
    assert_eq!(
        second.unmount(&spec, &mounted).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    first.unmount(&spec, &mounted).unwrap();
    fs::create_dir(dir.path().join("parent/readonly")).unwrap();
    let mut read_spec = spec.clone();
    read_spec.identity.root_id = "readonly".into();
    read_spec.target = dir_id(&dir.path().join("parent/readonly"));
    first
        .prepare(
            read_spec.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("readonly"),
            MountPolicy {
                read_only: true,
                no_exec: true,
            },
        )
        .unwrap();
    let read_mount = first.bind(&read_spec).unwrap();
    assert_eq!(
        fs::write(dir.path().join("parent/readonly/blocked"), b"x")
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EROFS)
    );
    let info = afs::node::vfs::ownerfs::native::MountInfo::parse(
        &fs::read("/proc/thread-self/mountinfo").unwrap(),
    )
    .unwrap();
    let ours = info
        .iter()
        .find(|m| m.mount_id == read_mount.mount_id)
        .unwrap();
    for flag in ["nosuid", "nodev", "noexec", "ro"] {
        assert!(
            ours.mount_options.iter().any(|v| v == flag),
            "missing flag {flag}"
        );
    }
    first.unmount(&read_spec, &read_mount).unwrap();
    assert!(!dir.path().join("source/blocked").exists());
}
