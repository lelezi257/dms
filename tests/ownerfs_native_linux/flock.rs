//! Actual kernel FUSE/native flock lane. This is not network P2P acceptance.
use super::{dir_id, ownerfs_fixture};
use afs::node::vfs::ownerfs::{
    native::{
        LinuxMountBackend, MountPolicy, NativeMountManager, NativeState, WorkspaceIdentity,
        WorkspaceMount,
    },
    root::{RootRight, root_id_from_name},
};
use std::{
    ffi::OsStr,
    fs::{self, File, TryLockError},
    io,
};

fn blocked(file: &File) -> io::Result<()> {
    match file.try_lock() {
        Err(TryLockError::WouldBlock) => Ok(()),
        Err(TryLockError::Error(error)) => Err(error),
        Ok(()) => {
            file.unlock()?;
            Err(io::Error::other(
                "exclusive flock succeeded while another description held a conflicting lock",
            ))
        }
    }
}

fn exclusive(file: &File) -> io::Result<()> {
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => io::Error::from_raw_os_error(libc::EAGAIN),
        TryLockError::Error(error) => error,
    })
}

fn shared(file: &File) -> io::Result<()> {
    file.try_lock_shared().map_err(|error| match error {
        TryLockError::WouldBlock => io::Error::from_raw_os_error(libc::EAGAIN),
        TryLockError::Error(error) => error,
    })
}

pub fn run() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let native_eligible = std::env::var("AFS_NATIVE_ELIGIBLE_CACHE").as_deref() != Ok("0");
    let dir = tempfile::tempdir().unwrap();
    let mount_path = dir.path().join("ownerfs");
    let peer_mount = dir.path().join("second-fuse");
    fs::create_dir(&mount_path).unwrap();
    fs::create_dir(&peer_mount).unwrap();
    let (disk, roots, ownerfs) =
        ownerfs_fixture::ownerfs_fixture_with_cache(&dir.path().join("data"), native_eligible);
    let session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &mount_path).unwrap();
    let second_session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &peer_mount).unwrap();
    let target = mount_path.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file"), b"old").unwrap();
    let authority = roots
        .enter_root(
            &root_id_from_name(OsStr::new("agent1")).unwrap(),
            RootRight::Write,
        )
        .unwrap();
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
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    backend
        .prepare(
            spec.clone(),
            File::open(&source).unwrap(),
            File::open(&mount_path).unwrap(),
            OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let manager = NativeMountManager::new(namespace, 8, backend).unwrap();
    manager.register(spec.clone()).unwrap();
    let active = manager.activate(&spec.identity).unwrap();
    assert_eq!(active.state, NativeState::NativeActive);
    assert_eq!(dir_id(&target), spec.source);
    let fuse_path = peer_mount.join("agent1/file");
    let results = (|| -> io::Result<()> {
        let native = File::options()
            .read(true)
            .write(true)
            .open(target.join("file"))?;
        let fuse = File::options().read(true).write(true).open(&fuse_path)?;
        let another_fuse = File::options().read(true).write(true).open(&fuse_path)?;
        exclusive(&native)?;
        let native_blocks_fuse = blocked(&fuse);
        native.unlock()?;
        native_blocks_fuse?;
        exclusive(&fuse)?;
        let fuse_blocks_native = blocked(&native);
        let fuse_blocks_second = blocked(&another_fuse);
        fuse.unlock()?;
        fuse_blocks_native?;
        fuse_blocks_second?;

        shared(&native)?;
        shared(&fuse)?;
        let upgrade = blocked(&fuse);
        native.unlock()?;
        upgrade?;
        // The failed Linux NB upgrade drops the prior FUSE shared flock.
        exclusive(&native)?;
        native.unlock()?;

        exclusive(&fuse)?;
        fs::rename(target.join("file"), target.join("old"))?;
        fs::write(target.join("file"), b"replacement")?;
        let replacement = File::options()
            .read(true)
            .write(true)
            .open(target.join("file"))?;
        exclusive(&replacement)?;
        let old_still_locked = blocked(&native);
        replacement.unlock()?;
        fuse.unlock()?;
        old_still_locked?;
        fs::remove_file(target.join("old"))?;
        exclusive(&fuse)?;
        let unlinked_still_locked = blocked(&native);
        fuse.unlock()?;
        unlinked_still_locked?;
        exclusive(&native)?;
        native.unlock()?;
        println!(
            "native_flock_kernel: native_vs_fuse=true fuse_vs_native=true independent_fuse=true shared=true failed_upgrade_matches_linux=true rename_replacement=true unlink_identity=true"
        );
        Ok(())
    })();
    // Close every native reference before normal detach; join both FUSE
    // sessions before asserting behavior, including the expected RED control.
    manager.quiesce(&spec.identity).unwrap();
    manager.detach(&spec.identity).unwrap();
    second_session.join().unwrap();
    session.join().unwrap();
    println!(
        "namespace={namespace:?} source={:?} observed={:?} native_eligible={native_eligible} result={results:?}",
        spec.source, active.observed
    );
    results.unwrap();
}
