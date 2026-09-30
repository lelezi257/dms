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
