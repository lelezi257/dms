//! A1 request-path observation, not Node/P2P or performance qualification.
//! Use the production FUSE adapter; an external strace observer counts actual
//! /dev/fuse requests between the controlled child actor's phase markers.
#![cfg(feature = "ownerfs")]

#[allow(dead_code)]
#[path = "ownerfs_native_linux/ownerfs_fixture.rs"]
mod ownerfs_fixture;

use afs::node::vfs::ownerfs::{
    native::{
        DirectoryIdentity, LinuxMountBackend, MountPolicy, NativeMountManager, NativeState,
        WorkspaceIdentity, WorkspaceMount,
    },
    root::{RootRight, root_id_from_name},
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
    process::{Command, Stdio},
};

fn identity(path: &Path) -> DirectoryIdentity {
    let stat = fs::metadata(path).unwrap();
    DirectoryIdentity {
        device: stat.dev(),
        inode: stat.ino(),
    }
}

#[test]
#[ignore = "A1 architecture observation; run with strace in private VM ext4 namespace"]
fn privileged_native_path_request_probe() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let mount = temp.path().join("ownerfs");
    fs::create_dir(&mount).unwrap();
    let (disk, roots, ownerfs) =
        ownerfs_fixture::ownerfs_fixture_with_cache(&temp.path().join("data"), true);
    let session = afs::node::fuse::mount_ownerfs(ownerfs, &mount).unwrap();
    let target = mount.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("warm"), b"path probe data").unwrap();
    // Keep the covered FUSE object for the positive control, rather than
    // reopening the target name after the native mount changes its meaning.
    let covered = File::open(&target).unwrap();
    let covered_identity = identity(&target);
    let root = root_id_from_name(OsStr::new("agent1")).unwrap();
    let authority = roots.enter_root(&root, RootRight::Write).unwrap();
    let source = disk.root_path().join(authority.data_dir().as_path());
    let source_identity = identity(&source);
    assert_ne!(source_identity.device, covered_identity.device);
    let grant = authority.grant();
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let spec = WorkspaceMount {
        identity: WorkspaceIdentity {
            root_id: grant.id.0.clone(),
            epoch: grant.epoch,
            home_node_id: grant.home_node_id.clone(),
            home_session_id: grant.home_session_id.clone(),
            namespace,
        },
        source: source_identity,
        target: covered_identity,
    };
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    backend
        .prepare(
            spec.clone(),
            File::open(&source).unwrap(),
            File::open(&mount).unwrap(),
            OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let manager = NativeMountManager::new(namespace, 8, backend).unwrap();
    manager.register(spec.clone()).unwrap();
    let ready = manager.activate(&spec.identity).unwrap();
    assert_eq!(ready.state, NativeState::NativeActive);
    assert_eq!(identity(&target), source_identity);
    let old_reference = format!("/proc/{}/fd/{}", std::process::id(), covered.as_raw_fd());
    // The management test launches this actor only after observed readiness.
    // It inherits the final mount namespace. No sleeps establish ordering.
    let actor = Command::new("python3")
        .args(["-I", "-c", ACTOR])
        .arg(&target)
        .arg(&source)
        .arg(old_reference)
        .arg(serde_json::to_string(&[source_identity.device, source_identity.inode]).unwrap())
        .stderr(Stdio::inherit())
        .output()
        .unwrap();
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap().state,
        NativeState::Detached
    );
    drop(manager);
    drop(authority);
    drop(covered);
    assert!(
        Command::new("umount")
            .arg(&mount)
            .status()
            .unwrap()
            .success()
    );
    session.join().unwrap();
    assert!(actor.status.success(), "actor failed: {:?}", actor.status);
    let observed: serde_json::Value = serde_json::from_slice(&actor.stdout).unwrap();
    assert_eq!(
        observed["namespace"],
        fs::read_link("/proc/self/ns/mnt")
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(observed["phase_count"], 5);
    assert_eq!(observed["iterations_per_phase"], 16);
    println!("native_path_probe {observed}");
}

const ACTOR: &str = r#"
import json, os, sys
target, backing, old_reference = sys.argv[1:4]
expected = json.loads(sys.argv[4])
native = os.open(target, os.O_RDONLY | os.O_DIRECTORY)
direct = os.open(backing, os.O_RDONLY | os.O_DIRECTORY)
old = os.open(old_reference, os.O_RDONLY | os.O_DIRECTORY)
def inode(fd):
    s = os.fstat(fd)
    return [s.st_dev, s.st_ino]
assert inode(native) == inode(direct) == expected
assert inode(old)[0] != expected[0]
pid = os.getpid()
def marker(kind, phase):
    data = ('DMS_PATH_PHASE_' + kind + '|' + phase + '|' + str(pid) + '\n').encode()
    assert os.write(2, data) == len(data)
def workload(phase, rootfd, prefix=''):
    marker('BEGIN', phase)
    for i in range(16):
        warm = prefix + 'warm'
        fd = os.open(warm, os.O_RDONLY, dir_fd=rootfd)
        assert os.read(fd, 64) == b'path probe data'
        os.close(fd)
        assert os.stat(warm, dir_fd=rootfd).st_size == 15
        name = prefix + phase + '-' + str(i)
        fd = os.open(name, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600, dir_fd=rootfd)
        assert os.write(fd, b'new') == 3
        os.close(fd)
        os.rename(name, name + '-renamed', src_dir_fd=rootfd, dst_dir_fd=rootfd)
        os.unlink(name + '-renamed', dir_fd=rootfd)
    marker('END', phase)
workload('absolute', None, target + '/')
os.chdir(target)
workload('native_cwd', None)
workload('native_dirfd', native)
workload('direct_backing_dirfd', direct)
# Put the FUSE control last so asynchronous FUSE RELEASE cannot be attributed
# to a subsequent native phase. Phase-end metadata/data calls are synchronous.
workload('old_fuse_dirfd', old)
assert sorted(os.listdir(native)) == sorted(os.listdir(old)) == ['warm']
print(json.dumps({'actor_pid':pid, 'phase_count':5, 'iterations_per_phase':16,
    'namespace':os.readlink('/proc/self/ns/mnt'), 'native_inode':inode(native),
    'covered_fuse_inode':inode(old), 'scope':'request-path observation, no timing/P2P qualification'}))
for fd in (old, direct, native): os.close(fd)
"#;
