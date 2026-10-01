//! A1 real-kernel failed/delayed transition combination, not Node/P2P READY.
//! Production adapter, native-eligible policy and current Home export authority.
#![cfg(feature = "ownerfs")]

#[allow(dead_code)]
#[path = "ownerfs_native_linux/ownerfs_fixture.rs"]
mod ownerfs_fixture;

use afs::node::vfs::ownerfs::{
    native::{
        LinuxMountBackend, MountPolicy, NativeMountManager, NativeState, workspace_event_channel,
    },
    root::{RootRight, root_id_from_name},
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
    process::Command,
    sync::Arc,
};

#[test]
#[ignore = "A1 transition mechanism; private VM FUSE/ext4 and CAP_SYS_ADMIN"]
fn privileged_native_failed_transition_preserves_backing() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let mount = temp.path().join("ownerfs");
    fs::create_dir(&mount).unwrap();
    let (disk, roots, owner) =
        ownerfs_fixture::ownerfs_fixture_with_cache(&temp.path().join("data"), true);
    let (sender, receiver) = workspace_event_channel(1).unwrap();
    owner.register_workspace_events(sender).unwrap();
    let session = afs::node::fuse::mount_ownerfs(owner.clone(), &mount).unwrap();
    let target = mount.join("agent1");
    fs::create_dir(&target).unwrap();
    // A following callback completes before consuming the post-reply hint;
    // no fixed wait or mount worker is required for transition I/O.
    let mut retained_file = File::create(target.join("retained")).unwrap();
    let hint = receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(hint.name, OsStr::new("agent1"));
    let old_directory = File::open(&target).unwrap();
    let covered_device = old_directory.metadata().unwrap().dev();
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let permit = Arc::new(
        owner
            .native_home_export(OsStr::new("agent1"), namespace)
            .unwrap(),
    );
    let root = root_id_from_name(OsStr::new("agent1")).unwrap();
    let admitted = roots.enter_root(&root, RootRight::Write).unwrap();
    let source = disk.root_path().join(admitted.data_dir().as_path());
    drop(admitted);
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    let spec = backend
        .prepare_for_home(
            &owner,
            &permit,
            File::open(&mount).unwrap(),
            MountPolicy::default(),
        )
        .unwrap();
    let manager = NativeMountManager::new(namespace, 8, backend).unwrap();
    assert_eq!(
        manager.register(spec.clone()).unwrap().state,
        NativeState::FuseReady
    );

    // The manager intentionally has not activated. Old-directory operations
    // run through the real FUSE channel and land on the prepared Home object.
    write_phase(
        "delayed",
        &old_directory,
        &mut retained_file,
        &source,
        covered_device,
    );
    assert_eq!(
        manager
            .status(&spec.identity.root_id)
            .unwrap()
            .unwrap()
            .state,
        NativeState::FuseReady
    );

    // Change only this test thread's effective capability; daemon threads and
    // unrelated VM processes are untouched. Native mount clone is denied by
    // the real kernel. Restore immediately, before checking the failed result.
    let mut capabilities = MountCapabilityGuard::disable().unwrap();
    let failure = manager.activate_for_home(&owner, &permit);
    capabilities.restore().unwrap();
    let errno = afs::error::errno(&failure.unwrap_err());
    let failed_state = manager.status(&spec.identity.root_id).unwrap().unwrap();
    assert_eq!(errno, libc::EPERM);
    assert_eq!(failed_state.state, NativeState::FuseOnly);
    assert!(failed_state.observed.is_none());
    assert_eq!(fs::metadata(&target).unwrap().dev(), covered_device);
    write_phase(
        "failed",
        &old_directory,
        &mut retained_file,
        &source,
        covered_device,
    );
    assert_eq!(fs::read(target.join("failed.txt")).unwrap(), b"failed-data");

    let active = manager.activate_for_home(&owner, &permit).unwrap();
    assert_eq!(active.state, NativeState::NativeActive);
    let exported = fs::metadata(&target).unwrap();
    assert_eq!(
        (exported.dev(), exported.ino()),
        (spec.source.device, spec.source.inode)
    );
    assert_ne!(exported.dev(), covered_device);
    // An old FUSE file/dirfd remains FUSE; new operations through that reference
    // are legal and must affect the same Home object after native activation.
    write_phase(
        "mounted",
        &old_directory,
        &mut retained_file,
        &source,
        covered_device,
    );
    for phase in ["delayed", "failed", "mounted"] {
        assert_eq!(
            fs::read(target.join(format!("{phase}.txt"))).unwrap(),
            format!("{phase}-data").as_bytes()
        );
    }
    assert_eq!(
        fs::read(target.join("retained")).unwrap(),
        b"mounted-retained"
    );
    let actor = Command::new("python3").args(["-I", "-c", r#"
import json, os, sys
expected = json.loads(sys.argv[1])
s = os.stat('.')
assert [s.st_dev, s.st_ino] == expected
with open('agent-ready.txt', 'wb') as f: f.write(b'native-ready')
print(json.dumps({'pid':os.getpid(), 'namespace':os.readlink('/proc/self/ns/mnt'), 'inode':expected}))
"#]).arg(serde_json::to_string(&[spec.source.device, spec.source.inode]).unwrap())
        .current_dir(&target).output().unwrap();
    assert!(
        actor.status.success(),
        "{}",
        String::from_utf8_lossy(&actor.stderr)
    );
    let actor: serde_json::Value = serde_json::from_slice(&actor.stdout).unwrap();
    assert_eq!(
        actor["namespace"],
        fs::read_link("/proc/self/ns/mnt")
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(
        fs::read(source.join("agent-ready.txt")).unwrap(),
        b"native-ready"
    );
    retained_file.sync_all().unwrap();
    drop(retained_file);
    drop(old_directory);
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap().state,
        NativeState::Detached
    );
    assert_eq!(
        fs::read(target.join("retained")).unwrap(),
        b"mounted-retained"
    );
    assert_eq!(
        fs::read(target.join("agent-ready.txt")).unwrap(),
        b"native-ready"
    );
    drop(manager);
    drop(permit);
    assert!(
        Command::new("umount")
            .arg(&mount)
            .status()
            .unwrap()
            .success()
    );
    session.join().unwrap();
    println!(
        "native_transition_probe {}",
        serde_json::json!({
            "mount_failure_errno":errno, "failure_state":format!("{:?}", failed_state.state),
            "failure_observed_mount":false, "covered_device":covered_device,
            "source_device":spec.source.device, "source_inode":spec.source.inode,
            "phases":["delayed", "failed", "mounted", "normal_detach_fallback"],
            "same_backing_verified":true, "capability_restored":true, "ready_actor":actor,
            "scope":"A1 current Home/FUSE/kernel combination, not Node/P2P READY or paused in-flight callback"
        })
    );
}

fn write_phase(
    phase: &str,
    old_directory: &File,
    retained_file: &mut File,
    source: &Path,
    covered_device: u64,
) {
    let path = format!("/proc/self/fd/{}/{}.txt", old_directory.as_raw_fd(), phase);
    let mut file = File::create(path).unwrap();
    assert_eq!(file.metadata().unwrap().dev(), covered_device);
    file.write_all(format!("{phase}-data").as_bytes()).unwrap();
    file.sync_all().unwrap();
    drop(file);
    retained_file.set_len(0).unwrap();
    retained_file.seek(SeekFrom::Start(0)).unwrap();
    retained_file
        .write_all(format!("{phase}-retained").as_bytes())
        .unwrap();
    retained_file.sync_all().unwrap();
    assert_eq!(retained_file.metadata().unwrap().dev(), covered_device);
    assert_eq!(
        fs::read(source.join(format!("{phase}.txt"))).unwrap(),
        format!("{phase}-data").as_bytes()
    );
    assert_eq!(
        fs::read(source.join("retained")).unwrap(),
        format!("{phase}-retained").as_bytes()
    );
}

#[repr(C)]
struct CapabilityHeader {
    version: u32,
    pid: i32,
}
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct CapabilityData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}
struct MountCapabilityGuard {
    saved: Option<[CapabilityData; 2]>,
}

impl MountCapabilityGuard {
    #[allow(unsafe_code)]
    fn disable() -> io::Result<Self> {
        let mut header = CapabilityHeader {
            version: 0x2008_0522,
            pid: 0,
        };
        let mut data = [CapabilityData::default(); 2];
        // SAFETY: Linux v3 capability header and two live writable data structs;
        // pid0 addresses only the calling thread, not another VM process.
        if unsafe { libc::syscall(libc::SYS_capget, &mut header, data.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let saved = data;
        assert_ne!(
            saved[0].effective & (1 << 21),
            0,
            "CAP_SYS_ADMIN must initially be effective"
        );
        data[0].effective &= !(1 << 21);
        set_capabilities(&data)?;
        Ok(Self { saved: Some(saved) })
    }
    fn restore(&mut self) -> io::Result<()> {
        if let Some(saved) = self.saved.as_ref() {
            set_capabilities(saved)?;
            self.saved = None;
        }
        Ok(())
    }
}
impl Drop for MountCapabilityGuard {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("test thread capability restoration failed: {error}");
        }
    }
}
#[allow(unsafe_code)]
fn set_capabilities(data: &[CapabilityData; 2]) -> io::Result<()> {
    let header = CapabilityHeader {
        version: 0x2008_0522,
        pid: 0,
    };
    // SAFETY: Correct v3 header and two readable structs remain live; permitted
    // and inheritable sets are retained, so restoration remains possible.
    if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
