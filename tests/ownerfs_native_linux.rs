#![cfg(feature = "ownerfs")]
use afs::node::vfs::ownerfs::native::LinuxMountBackend;
use afs::node::vfs::ownerfs::native::MountPolicy;
use afs::node::vfs::ownerfs::native::{
    DirectoryIdentity, MountBackend, NativeMountManager, NativeState, WorkspaceIdentity,
    WorkspaceMount,
};
#[path = "ownerfs_native_linux/ownerfs_fixture.rs"]
mod ownerfs_fixture;
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

struct CrashBackend {
    inner: LinuxMountBackend,
    stage: String,
}
impl MountBackend for CrashBackend {
    fn inspect(
        &self,
        spec: &WorkspaceMount,
    ) -> std::io::Result<Option<afs::node::vfs::ownerfs::native::MountIdentity>> {
        self.inner.inspect(spec)
    }
    fn bind(
        &self,
        _: &WorkspaceMount,
    ) -> std::io::Result<afs::node::vfs::ownerfs::native::MountIdentity> {
        panic!("crash actor requires journaled path")
    }
    fn bind_journaled(
        &self,
        spec: &WorkspaceMount,
        callback: &mut dyn FnMut(
            &afs::node::vfs::ownerfs::native::MountIdentity,
        ) -> std::io::Result<()>,
    ) -> std::io::Result<afs::node::vfs::ownerfs::native::MountIdentity> {
        let mount = self.inner.bind_journaled(spec, &mut |claim| {
            callback(claim)?;
            if self.stage == "before-attach" {
                std::process::exit(91);
            }
            Ok(())
        })?;
        if self.stage == "after-attach" {
            std::process::exit(92);
        }
        Ok(mount)
    }
    fn attached_claim(
        &self,
        spec: &WorkspaceMount,
    ) -> std::io::Result<Option<afs::node::vfs::ownerfs::native::MountIdentity>> {
        self.inner.attached_claim(spec)
    }
    fn unmount(
        &self,
        spec: &WorkspaceMount,
        mount: &afs::node::vfs::ownerfs::native::MountIdentity,
    ) -> std::io::Result<()> {
        self.inner.unmount(spec, mount)?;
        if self.stage == "after-unmount" {
            std::process::exit(93);
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires real process exit, private mount namespace, Linux >= 6.8, VM ext4"]
fn privileged_native_crash_recovery() {
    use afs::node::vfs::ownerfs::native::{JournalSnapshot, MountJournal};
    use std::ffi::OsStr;
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    if let Ok(stage) = std::env::var("AFS_NATIVE_CRASH_STAGE") {
        let path = std::path::PathBuf::from(std::env::var_os("AFS_NATIVE_CRASH_DIR").unwrap());
        let spec: WorkspaceMount =
            serde_json::from_slice(&fs::read(path.join("spec.json")).unwrap()).unwrap();
        println!(
            "crash_actor_pid={} stage={stage} ns={:?}",
            std::process::id(),
            LinuxMountBackend::current_namespace().unwrap()
        );
        let inner = LinuxMountBackend::new(spec.identity.namespace, 8).unwrap();
        inner
            .prepare(
                spec.clone(),
                File::open(path.join("source")).unwrap(),
                File::open(path.join("parent")).unwrap(),
                OsStr::new("agent1"),
                MountPolicy::default(),
            )
            .unwrap();
        let backend = CrashBackend { inner, stage };
        let journal = MountJournal::open(
            File::open(path.join("journal")).unwrap(),
            boot.trim(),
            spec.identity.namespace,
            8,
        )
        .unwrap();
        let manager =
            NativeMountManager::with_journal(spec.identity.namespace, 8, backend, journal).unwrap();
        manager.register(spec.clone()).unwrap();
        manager.activate(&spec.identity).unwrap();
        manager.quiesce(&spec.identity).unwrap();
        manager.detach(&spec.identity).unwrap();
        panic!("configured crash point was not executed");
    }
    for (stage, exit) in [
        ("before-attach", 91),
        ("after-attach", 92),
        ("after-unmount", 93),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (unused, spec) = setup(&dir);
        drop(unused);
        fs::create_dir(dir.path().join("journal")).unwrap();
        fs::write(dir.path().join("source/file"), b"data survives daemon exit").unwrap();
        fs::write(
            dir.path().join("spec.json"),
            serde_json::to_vec(&spec).unwrap(),
        )
        .unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "privileged_native_crash_recovery",
                "--nocapture",
            ])
            .env("AFS_NATIVE_CRASH_STAGE", stage)
            .env("AFS_NATIVE_CRASH_DIR", dir.path())
            .status()
            .unwrap();
        assert_eq!(result.code(), Some(exit));
        let snapshot: JournalSnapshot =
            serde_json::from_slice(&fs::read(dir.path().join("journal/state.json")).unwrap())
                .unwrap();
        let saved = &snapshot.records[0];
        assert_eq!(
            saved.status.state,
            if stage == "after-unmount" {
                NativeState::Unmounting
            } else {
                NativeState::Mounting
            }
        );
        let claim = saved.owned_mount.as_ref().unwrap();
        println!("restart stage={stage} persisted_claim={claim:?}");
        let backend = LinuxMountBackend::new(spec.identity.namespace, 8).unwrap();
        backend
            .prepare_recovery(
                spec.clone(),
                File::open(dir.path().join("source")).unwrap(),
                File::open(dir.path().join("parent")).unwrap(),
                OsStr::new("agent1"),
                MountPolicy::default(),
                claim,
            )
            .unwrap();
        let journal = MountJournal::open(
            File::open(dir.path().join("journal")).unwrap(),
            boot.trim(),
            spec.identity.namespace,
            8,
        )
        .unwrap();
        let manager =
            NativeMountManager::with_journal(spec.identity.namespace, 8, backend, journal).unwrap();
        assert_eq!(
            manager.status("root1").unwrap().unwrap().state,
            NativeState::Recovering
        );
        assert!(manager.activate(&spec.identity).is_err());
        let recovered = manager.reconcile(&spec).unwrap();
        assert_eq!(
            recovered.state,
            match stage {
                "before-attach" => NativeState::FuseReady,
                "after-attach" => NativeState::NativeActive,
                _ => NativeState::Detached,
            }
        );
        if stage == "after-attach" {
            assert_eq!(recovered.observed.as_ref(), Some(claim));
            assert_eq!(
                fs::read(dir.path().join("parent/agent1/file")).unwrap(),
                b"data survives daemon exit"
            );
            manager.quiesce(&spec.identity).unwrap();
            manager.detach(&spec.identity).unwrap();
        }
        assert_eq!(dir_id(&dir.path().join("parent/agent1")), spec.target);
        assert_eq!(
            fs::read(dir.path().join("source/file")).unwrap(),
            b"data survives daemon exit"
        );
        drop(manager);
    }
}

#[test]
#[ignore = "requires CAP_SYS_ADMIN, isolated VM namespace and Linux >= 6.8"]
fn privileged_existing_mount_cannot_be_registered_as_covered_directory() {
    use std::ffi::OsStr;
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let (unused, spec) = setup(&dir);
    drop(unused);
    let target = dir.path().join("parent/agent1");
    assert!(
        std::process::Command::new("mount")
            .arg("--bind")
            .arg(&target)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    let backend = LinuxMountBackend::new(spec.identity.namespace, 8).unwrap();
    let result = backend.prepare(
        spec,
        File::open(dir.path().join("source")).unwrap(),
        File::open(dir.path().join("parent")).unwrap(),
        OsStr::new("agent1"),
        MountPolicy::default(),
    );
    drop(backend);
    assert!(
        std::process::Command::new("umount")
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ESTALE));
}

#[test]
#[ignore = "requires actual helper crash, private VM namespace and Linux >= 6.8"]
fn privileged_recovery_does_not_admit_rw_export_under_new_readonly_policy() {
    use afs::node::vfs::ownerfs::native::{JournalSnapshot, MountJournal};
    let dir = tempfile::tempdir().unwrap();
    let (unused, spec) = setup(&dir);
    drop(unused);
    fs::create_dir(dir.path().join("journal")).unwrap();
    fs::write(
        dir.path().join("spec.json"),
        serde_json::to_vec(&spec).unwrap(),
    )
    .unwrap();
    let exit = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "privileged_native_crash_recovery",
            "--nocapture",
        ])
        .env("AFS_NATIVE_CRASH_STAGE", "after-attach")
        .env("AFS_NATIVE_CRASH_DIR", dir.path())
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(92));
    let snapshot: JournalSnapshot =
        serde_json::from_slice(&fs::read(dir.path().join("journal/state.json")).unwrap()).unwrap();
    let claim = snapshot.records[0].owned_mount.as_ref().unwrap();
    let backend = LinuxMountBackend::new(spec.identity.namespace, 8).unwrap();
    backend
        .prepare_recovery(
            spec.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy {
                read_only: true,
                no_exec: true,
            },
            claim,
        )
        .unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let journal = MountJournal::open(
        File::open(dir.path().join("journal")).unwrap(),
        boot.trim(),
        spec.identity.namespace,
        8,
    )
    .unwrap();
    let manager =
        NativeMountManager::with_journal(spec.identity.namespace, 8, backend, journal).unwrap();
    let result = manager.reconcile(&spec);
    // Verified ownership still permits quiesce/cleanup, never native admission.
    manager.quiesce(&spec.identity).unwrap();
    manager.detach(&spec.identity).unwrap();
    assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EPERM));
}

#[test]
#[ignore = "requires CAP_SYS_ADMIN, private VM namespace and Linux >= 6.8"]
fn privileged_release_retains_live_claim_and_fences_reprepared_epoch() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let (backend, first) = setup(&dir);
    let mount = backend.bind(&first).unwrap();
    assert_eq!(
        backend.release_prepared(&first).unwrap_err().raw_os_error(),
        Some(libc::EBUSY)
    );
    assert_eq!(backend.inspect(&first).unwrap(), Some(mount.clone()));
    backend.unmount(&first, &mount).unwrap();
    backend.release_prepared(&first).unwrap();
    backend.release_prepared(&first).unwrap();
    assert_eq!(
        backend.inspect(&first).unwrap_err().raw_os_error(),
        Some(libc::ENOENT)
    );

    let mut next = first.clone();
    next.identity.epoch += 1;
    next.identity.home_session_id = "new-home-session".into();
    backend
        .prepare(
            next.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    assert_eq!(
        backend.release_prepared(&first).unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
    let next_mount = backend.bind(&next).unwrap();
    assert_eq!(backend.inspect(&next).unwrap(), Some(next_mount.clone()));
    backend.unmount(&next, &next_mount).unwrap();
    backend.release_prepared(&next).unwrap();
    assert!(dir.path().join("source").is_dir());

    // Removing a foreign cover must never be a side effect of releasing pins.
    backend
        .prepare(
            next.clone(),
            File::open(dir.path().join("source")).unwrap(),
            File::open(dir.path().join("parent")).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let target = dir.path().join("parent/agent1");
    assert!(
        std::process::Command::new("mount")
            .arg("--bind")
            .arg(&target)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    let observed = backend.inspect(&next).unwrap();
    let refusal = backend.release_prepared(&next).unwrap_err();
    assert_eq!(backend.inspect(&next).unwrap(), observed);
    assert!(
        std::process::Command::new("umount")
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(refusal.raw_os_error(), Some(libc::EBUSY));
    backend.release_prepared(&next).unwrap();
}

#[test]
#[ignore = "requires real /dev/fuse, CAP_SYS_ADMIN, private VM namespace and Linux >= 6.8"]
fn privileged_fd_mount_over_real_ownerfs_directory() {
    use afs::node::vfs::ownerfs::root::{RootRight, root_id_from_name};
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let mount_path = dir.path().join("ownerfs");
    fs::create_dir(&mount_path).unwrap();
    let (disk, roots, ownerfs) = ownerfs_fixture::ownerfs_fixture(&dir.path().join("data"));
    let session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &mount_path).unwrap();
    // mkdir is a real kernel FUSE request; it completes before FD preparation.
    let target = mount_path.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("transition.txt"), b"FUSE transition").unwrap();
    let id = root_id_from_name(std::ffi::OsStr::new("agent1")).unwrap();
    let authority = roots.enter_root(&id, RootRight::Write).unwrap();
    let source = disk.root_path().join(authority.data_dir().as_path());
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let grant = authority.grant();
    let spec = WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: grant.id.0.clone(),
            epoch: grant.epoch,
            home_node_id: grant.home_node_id.clone(),
            home_session_id: grant.home_session_id.clone(),
            namespace,
        },
        source: dir_id(&source),
        target: dir_id(&target),
    };
    assert_ne!(spec.source.device, spec.target.device);
    let old_directory = File::open(&target).unwrap();
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    backend
        .prepare(
            spec.clone(),
            File::open(&source).unwrap(),
            File::open(&mount_path).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let mounted = backend.bind(&spec).unwrap();
    assert_eq!(dir_id(&target), spec.source);
    assert_eq!(old_directory.metadata().unwrap().dev(), spec.target.device);
    assert_eq!(
        fs::read(target.join("transition.txt")).unwrap(),
        b"FUSE transition"
    );
    fs::write(target.join("native.txt"), b"native on same backing").unwrap();
    assert_eq!(
        fs::read(source.join("native.txt")).unwrap(),
        b"native on same backing"
    );
    // Restart preparation uses a detached parent clone while the real FUSE
    // daemon lives; it must find the covered FUSE inode, not the native inode.
    let recovered = LinuxMountBackend::new(namespace, 8).unwrap();
    recovered
        .prepare_recovery(
            spec.clone(),
            File::open(&source).unwrap(),
            File::open(&mount_path).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
            &mounted,
        )
        .unwrap();
    recovered.adopt_verified_claim(&spec, &mounted).unwrap();
    recovered.verify_policy(&spec, &mounted).unwrap();
    recovered.unmount(&spec, &mounted).unwrap();
    recovered.release_prepared(&spec).unwrap();
    assert_eq!(dir_id(&target), spec.target);
    assert_eq!(
        fs::read(target.join("native.txt")).unwrap(),
        b"native on same backing"
    );
    println!(
        "real_ownerfs_mount source={:?} covered={:?} mount={:?}",
        spec.source, spec.target, mounted
    );
    drop(recovered);
    // The original helper retains an unresolved claim after external removal;
    // dropping its preparation is teardown, never product reclamation proof.
    drop(backend);
    drop(old_directory);
    drop(authority);
    assert!(
        std::process::Command::new("umount")
            .arg(&mount_path)
            .status()
            .unwrap()
            .success()
    );
    drop(session);
    drop(ownerfs);
    drop(roots);
    drop(disk);
}

struct RetainedReference(std::process::Child);
impl Drop for RetainedReference {
    fn drop(&mut self) {
        // Only this unreaped child may be signaled. Closing its input requests
        // orderly exit; kill+wait also bounds teardown after a test panic.
        drop(self.0.stdin.take());
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires Python3, CAP_SYS_ADMIN, private VM namespace and Linux >= 6.8"]
fn privileged_native_retained_reference_busy_matrix() {
    use std::io::BufRead;
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let script = r#"
import os, sys, ctypes, json
mode, target = sys.argv[1:]
if mode == 'dirfd':
    retained = os.open(target, os.O_RDONLY | os.O_DIRECTORY)
elif mode == 'cwd':
    os.chdir(target)
else:
    libc = ctypes.CDLL(None, use_errno=True)
    libc.mmap.restype = ctypes.c_void_p
    libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
    opened = os.open(target + '/pages', os.O_RDONLY)
    pointer = libc.mmap(None, 4096, 1, 1 if mode == 'shared-mmap' else 2, opened, 0)
    assert pointer != ctypes.c_void_p(-1).value, ctypes.get_errno()
    assert ctypes.string_at(pointer, 4) == b'page'
    os.close(opened)
    # ctypes mmap holds only a VMA; unlike Python mmap there is no duplicate fd.
    assert all(not os.readlink('/proc/self/fd/' + name).endswith('/pages')
               for name in os.listdir('/proc/self/fd') if os.path.exists('/proc/self/fd/' + name))
print(json.dumps({'mode':mode, 'pid':os.getpid(), 'ready':True}), flush=True)
sys.stdin.buffer.read(1)
"#;
    for mode in ["dirfd", "cwd", "shared-mmap", "private-mmap"] {
        let dir = tempfile::tempdir().unwrap();
        let (backend, spec) = setup(&dir);
        let mut pages = vec![0_u8; 4096];
        pages[..4].copy_from_slice(b"page");
        fs::write(dir.path().join("source/pages"), pages).unwrap();
        let mounted = backend.bind(&spec).unwrap();
        let child = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(mode)
            .arg(dir.path().join("parent/agent1"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut retained = RetainedReference(child);
        let mut line = String::new();
        std::io::BufReader::new(retained.0.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["pid"].as_u64(), Some(u64::from(retained.0.id())));
        assert_eq!(ready["mode"], mode);
        assert_eq!(ready["ready"], true);
        let refused = backend.unmount(&spec, &mounted);
        assert_eq!(
            refused.unwrap_err().raw_os_error(),
            Some(libc::EBUSY),
            "mode={mode}"
        );
        assert_eq!(backend.inspect(&spec).unwrap(), Some(mounted.clone()));
        println!(
            "retained_reference mode={mode} child={} unmount=EBUSY",
            retained.0.id()
        );
        // Reap this exact actor before the successful normal-unmount retry.
        drop(retained);
        backend.unmount(&spec, &mounted).unwrap();
        backend.release_prepared(&spec).unwrap();
        assert_eq!(
            &fs::read(dir.path().join("source/pages")).unwrap()[..4],
            b"page"
        );
    }
}

#[test]
#[ignore = "requires CAP_SYS_ADMIN, private VM namespace, ext4 and Linux >= 6.8"]
fn privileged_journal_maintenance_preserves_active_export_and_data() {
    use afs::node::vfs::ownerfs::native::MountJournal;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let (backend, spec) = setup(&dir);
    fs::write(dir.path().join("source/file"), b"backing retained").unwrap();
    let journal_path = dir.path().join("journal");
    fs::create_dir(&journal_path).unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let journal = MountJournal::open(
        File::open(&journal_path).unwrap(),
        boot.trim(),
        spec.identity.namespace,
        8,
    )
    .unwrap();
    let manager =
        NativeMountManager::with_journal(spec.identity.namespace, 8, backend, journal).unwrap();
    manager.register(spec.clone()).unwrap();
    let active = manager.activate(&spec.identity).unwrap();
    let state_before = fs::read(journal_path.join("state.json")).unwrap();
    let orphan = journal_path.join(".state-123-1000");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&orphan)
        .unwrap();
    file.write_all(&state_before).unwrap();
    drop(file);
    let report = manager.cleanup_journal_orphans().unwrap();
    assert_eq!(report.removed, vec![".state-123-1000"]);
    assert!(!orphan.exists());
    assert_eq!(
        manager.activate(&spec.identity).unwrap().observed,
        active.observed
    );
    assert_eq!(
        fs::read(journal_path.join("state.json")).unwrap(),
        state_before
    );
    assert_eq!(
        fs::read(dir.path().join("parent/agent1/file")).unwrap(),
        b"backing retained"
    );
    manager.quiesce(&spec.identity).unwrap();
    manager.detach(&spec.identity).unwrap();
    assert_eq!(
        fs::read(dir.path().join("source/file")).unwrap(),
        b"backing retained"
    );
}

#[test]
#[ignore = "requires real /dev/fuse, CAP_SYS_ADMIN and private VM namespace"]
fn privileged_ownerfs_post_reply_workspace_hints_do_not_wait_for_consumer() {
    use afs::node::vfs::ownerfs::native::workspace_event_channel;
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let mount_path = dir.path().join("ownerfs");
    fs::create_dir(&mount_path).unwrap();
    let (disk, roots, ownerfs) = ownerfs_fixture::ownerfs_fixture(&dir.path().join("data"));
    let (sender, receiver) = workspace_event_channel(1).unwrap();
    ownerfs.register_workspace_events(sender).unwrap();
    let session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &mount_path).unwrap();
    // Consumer remains idle while two real root mkdir requests and a nested
    // mkdir complete. A blocking send would deadlock the second root creation.
    fs::create_dir(mount_path.join("agent1")).unwrap();
    fs::create_dir(mount_path.join("agent1/nested")).unwrap();
    fs::write(mount_path.join("agent1/nested/data"), b"still FUSE").unwrap();
    fs::create_dir(mount_path.join("agent2")).unwrap();
    // A following real FUSE create is a callback-order barrier: mkdir's reply
    // can reach its caller just before post-reply hint publication finishes.
    fs::write(mount_path.join("agent2/barrier"), b"callback passed").unwrap();
    let first = receiver.recv_timeout(std::time::Duration::from_secs(2));
    let overflow = receiver.rescan_required();
    let next = receiver.try_recv();
    let bytes = fs::read(mount_path.join("agent1/nested/data")).unwrap();
    // Always clean the real mount before asserting the missing-hook RED case.
    assert!(
        std::process::Command::new("umount")
            .arg(&mount_path)
            .status()
            .unwrap()
            .success()
    );
    drop(session);
    drop(ownerfs);
    drop(roots);
    drop(disk);
    let first = first.expect("successful top-level mkdir must emit a post-reply hint");
    assert_eq!(first.name, std::ffi::OsString::from("agent1"));
    assert_ne!(first.inode.value, 1);
    assert!(
        overflow,
        "second top-level hint must request authoritative rescan"
    );
    assert_eq!(next.unwrap_err(), std::sync::mpsc::TryRecvError::Empty);
    assert_eq!(bytes, b"still FUSE");
    assert!(receiver.begin_rescan());
}

#[test]
#[ignore = "requires real OwnerFs FUSE, Python3, private VM namespace and ext4; full-semantics RED case"]
fn privileged_old_fuse_directory_tracks_native_rename() {
    use afs::node::vfs::ownerfs::root::{RootRight, root_id_from_name};
    use std::io::{BufRead, Write};
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let mount_path = dir.path().join("ownerfs");
    fs::create_dir(&mount_path).unwrap();
    let (disk, roots, ownerfs) = ownerfs_fixture::ownerfs_fixture(&dir.path().join("data"));
    let session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &mount_path).unwrap();
    let target = mount_path.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::create_dir_all(target.join("left/moving")).unwrap();
    fs::create_dir(target.join("right")).unwrap();
    fs::write(target.join("left/parent-id"), b"LEFT").unwrap();
    fs::write(target.join("right/parent-id"), b"RIGHT").unwrap();
    fs::write(target.join("left/moving/data"), b"same directory").unwrap();
    let id = root_id_from_name(std::ffi::OsStr::new("agent1")).unwrap();
    let authority = roots.enter_root(&id, RootRight::Write).unwrap();
    let source = disk.root_path().join(authority.data_dir().as_path());
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let grant = authority.grant();
    let spec = WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: grant.id.0.clone(),
            epoch: grant.epoch,
            home_node_id: grant.home_node_id.clone(),
            home_session_id: grant.home_session_id.clone(),
            namespace,
        },
        source: dir_id(&source),
        target: dir_id(&target),
    };
    let script = r#"
import os, sys, json
os.chdir(sys.argv[1])
legacy = os.open('.', os.O_RDONLY | os.O_DIRECTORY)
native = os.open(sys.argv[2], os.O_RDONLY | os.O_DIRECTORY)
covered = os.open(sys.argv[3], os.O_RDONLY | os.O_DIRECTORY)
print(json.dumps({'ready': True, 'pid': os.getpid(), 'legacy_dir': [os.fstat(legacy).st_dev, os.fstat(legacy).st_ino], 'native_dir': [os.fstat(native).st_dev, os.fstat(native).st_ino]}), flush=True)
assert sys.stdin.readline().strip() == 'renamed'
def read_at(descriptor, relative):
    try:
        fd = os.open(relative, os.O_RDONLY, dir_fd=descriptor)
        try: return {'data': os.read(fd, 1024).decode('ascii')}
        finally: os.close(fd)
    except OSError as error: return {'errno': error.errno}
print(json.dumps({'legacy_data': read_at(legacy, 'data'), 'native_data': read_at(native, 'data'), 'legacy_parent': read_at(legacy, '../parent-id'), 'native_parent': read_at(native, '../parent-id'), 'cwd_parent': read_at(None, '../parent-id')}), flush=True)
assert sys.stdin.readline().strip() == 'relookup'
try:
    refreshed = os.stat('right/moving', dir_fd=covered)
    relookup = {'ino': refreshed.st_ino}
except OSError as error:
    relookup = {'errno': error.errno}
print(json.dumps({'relookup': relookup, 'legacy_data': read_at(legacy, 'data'), 'legacy_parent': read_at(legacy, '../parent-id'), 'cwd_parent': read_at(None, '../parent-id')}), flush=True)
os.close(legacy)
os.close(native)
os.close(covered)
"#;
    let mut actor = RetainedReference(
        std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(target.join("left/moving"))
            .arg(source.join("left/moving"))
            .arg(&target)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = std::io::BufReader::new(actor.0.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["ready"], true);
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    backend
        .prepare(
            spec.clone(),
            File::open(&source).unwrap(),
            File::open(&mount_path).unwrap(),
            std::ffi::OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let mounted = backend.bind(&spec).unwrap();
    fs::rename(target.join("left/moving"), target.join("right/moving")).unwrap();
    writeln!(actor.0.stdin.as_mut().unwrap(), "renamed").unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let observed: serde_json::Value = serde_json::from_str(&line).unwrap();
    // Diagnostic control only: force a lookup through the already-held covered
    // FUSE root, to distinguish kernel alias relocation from backend path state.
    // Correctness still requires the FIRST observation without any extra lookup.
    writeln!(actor.0.stdin.as_mut().unwrap(), "relookup").unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let alias_control: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(actor.0.wait().unwrap().success());
    // Cleanup before asserting the semantic RED, retaining the full observation.
    backend.unmount(&spec, &mounted).unwrap();
    backend.release_prepared(&spec).unwrap();
    drop(backend);
    drop(authority);
    assert!(
        std::process::Command::new("umount")
            .arg(&mount_path)
            .status()
            .unwrap()
            .success()
    );
    drop(session);
    drop(ownerfs);
    drop(roots);
    drop(disk);
    println!(
        "legacy_directory_ready={ready} legacy_directory_namespace={observed} forced_alias_control={alias_control}"
    );
    assert_eq!(
        observed["native_data"]["data"], "same directory",
        "native fd positive control"
    );
    assert_eq!(
        observed["native_parent"]["data"], "RIGHT",
        "native parent positive control"
    );
    assert_eq!(
        alias_control["legacy_data"], observed["native_data"],
        "fresh directory lookup must reconcile cached descendant paths"
    );
    assert_eq!(
        alias_control["legacy_parent"], observed["native_parent"],
        "forced lookup kernel alias relocation control"
    );
    assert_eq!(
        alias_control["cwd_parent"], observed["native_parent"],
        "forced lookup cwd alias relocation control"
    );
    assert_eq!(
        observed["legacy_data"], observed["native_data"],
        "old FUSE dirfd must retain the moved directory object"
    );
    assert_eq!(
        observed["legacy_parent"], observed["native_parent"],
        "old FUSE dirfd must observe its current parent"
    );
    assert_eq!(
        observed["cwd_parent"], observed["native_parent"],
        "old FUSE cwd must observe its current parent"
    );
}
