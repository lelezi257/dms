//! Current Home authority through real FUSE and normal native teardown.
//! In-process Meta fixture only: no production Node/Agent or network P2P claim.
use super::{dir_id, ownerfs_fixture};
use afs::node::vfs::ownerfs::{
    native::{
        LinuxMountBackend, MountInfo, MountJournal, MountPolicy, NativeMountManager, NativeState,
    },
    root::{RootRight, root_id_from_name},
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    process::Command,
    sync::Arc,
};

pub fn run() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let eligible = std::env::var("AFS_NATIVE_ELIGIBLE_CACHE").as_deref() != Ok("0");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ownerfs");
    fs::create_dir(&path).unwrap();
    let (disk, roots, owner) =
        ownerfs_fixture::ownerfs_fixture_with_cache(&dir.path().join("data"), eligible);
    let session = afs::node::fuse::mount_ownerfs(owner.clone(), &path).unwrap();
    let target = path.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file"), b"same Home backing").unwrap();
    let namespace = LinuxMountBackend::current_namespace().unwrap();
    let issued = owner.native_home_export(OsStr::new("agent1"), namespace);
    if !eligible {
        let rejected = issued.is_err();
        assert!(
            Command::new("umount")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        session.join().unwrap();
        assert!(
            rejected,
            "ordinary cached FUSE must not issue native authority"
        );
        println!("home_authority ordinary_cache_rejected=true");
        return;
    }
    let permit = Arc::new(issued.unwrap());
    let root_id = root_id_from_name(OsStr::new("agent1")).unwrap();
    let admitted = roots.enter_root(&root_id, RootRight::Write).unwrap();
    let source = disk.root_path().join(admitted.data_dir().as_path());
    drop(admitted);
    let mut old_fuse = File::open(target.join("file")).unwrap();
    let backend = LinuxMountBackend::new(namespace, 8).unwrap();
    let spec = backend
        .prepare_for_home(
            &owner,
            &permit,
            File::open(&path).unwrap(),
            MountPolicy::default(),
        )
        .unwrap();
    fs::create_dir(dir.path().join("journal")).unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let journal = MountJournal::open(
        File::open(dir.path().join("journal")).unwrap(),
        boot.trim(),
        namespace,
        8,
    )
    .unwrap();
    let manager = NativeMountManager::with_journal(namespace, 8, backend, journal).unwrap();
    manager.register(spec.clone()).unwrap();
    let active = manager.activate_for_home(&owner, &permit).unwrap();
    let results = (|| -> Result<serde_json::Value, String> {
        if active.state != NativeState::NativeActive || dir_id(&target) != spec.source {
            return Err("authorized export did not expose the exact Home object".into());
        }
        if fs::read(target.join("file")).map_err(|e| e.to_string())? != b"same Home backing" {
            return Err("wrong initial backing data".into());
        }
        let held = File::open(target.join("file")).map_err(|e| e.to_string())?;
        roots.revoke_root(&root_id);
        let claim_id = active.observed.as_ref().ok_or("missing claim")?.mount_id;
        let before_observation =
            MountInfo::parse(&fs::read("/proc/thread-self/mountinfo").map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let refused = manager.activate_for_home(&owner, &permit).is_err();
        let fuse_denied = old_fuse.read(&mut [0u8; 1]).is_err();
        let busy = manager
            .status(&spec.identity.root_id)
            .map_err(|e| e.to_string())?
            .ok_or("missing status")?;
        let after_observation =
            MountInfo::parse(&fs::read("/proc/thread-self/mountinfo").map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        println!(
            "home_revocation_observation before_present={} after_present={} held_file_len={:?} busy={busy:?}",
            before_observation
                .iter()
                .any(|mount| mount.mount_id == claim_id),
            after_observation
                .iter()
                .any(|mount| mount.mount_id == claim_id),
            held.metadata().map(|metadata| metadata.len())
        );
        drop(held);
        let detached = manager.detach(&spec.identity).map_err(|e| {
            format!(
                "normal teardown after Home revocation: {e}; status={:?}",
                manager.status(&spec.identity.root_id)
            )
        })?;
        if !refused
            || !fuse_denied
            || !after_observation
                .iter()
                .any(|mount| mount.mount_id == claim_id)
            || busy.state != NativeState::Draining
            || busy.last_errno != Some(libc::EBUSY)
            || detached.state != NativeState::Detached
        {
            return Err(format!(
                "invalid revoke/busy/teardown state: refused={refused} fuse_denied={fuse_denied} busy={busy:?} detached={detached:?}"
            ));
        }
        if fs::read(source.join("file")).map_err(|e| e.to_string())? != b"same Home backing" {
            return Err("teardown changed or removed backing data".into());
        }
        Ok(serde_json::json!({"busy":busy,"detached":detached,"source":spec.source}))
    })();
    drop(old_fuse);
    // Keep behavioral failure separate from cleanup. This private, exclusively
    // controlled fixture may remove only its original verified physical claim.
    let claim = active.observed.as_ref().unwrap();
    let mounts = MountInfo::parse(&fs::read("/proc/thread-self/mountinfo").unwrap()).unwrap();
    if mounts.iter().any(|entry| entry.mount_id == claim.mount_id) {
        assert_eq!(dir_id(&target), spec.source);
        assert!(
            Command::new("umount")
                .arg(&target)
                .status()
                .unwrap()
                .success()
        );
    }
    drop(manager);
    drop(permit);
    let retired_metadata_denied = fs::metadata(&target).is_err();
    assert!(
        Command::new("umount")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    session.join().unwrap();
    println!("home_authority result={results:?} initial_export={active:?}");
    assert!(
        results.is_ok(),
        "Home authority lifecycle failure: {results:?}"
    );
    assert!(
        retired_metadata_denied,
        "released mountpoint anchor must not retain revoked FUSE metadata access"
    );
}
