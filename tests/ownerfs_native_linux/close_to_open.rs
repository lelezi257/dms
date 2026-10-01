//! Native/independent Home FUSE-session contract regression. No network P2P
//! claim: both real kernel sessions reuse the fixture's authenticated backend.
use super::{dir_id, ownerfs_fixture};
use afs::node::vfs::ownerfs::{
    native::{
        LinuxMountBackend, MountJournal, MountPolicy, NativeMountManager, NativeState,
        WorkspaceIdentity, WorkspaceMount,
    },
    root::{RootRight, root_id_from_name},
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    process::Command,
};

fn observed(path: &std::path::Path) -> serde_json::Value {
    match fs::read(path) {
        Ok(bytes) => serde_json::json!({"bytes":bytes, "length":fs::metadata(path).unwrap().len()}),
        Err(error) => serde_json::json!({"errno":error.raw_os_error()}),
    }
}

pub fn run() {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let native_eligible = std::env::var("AFS_NATIVE_ELIGIBLE_CACHE").as_deref() != Ok("0");
    let dir = tempfile::tempdir().unwrap();
    let mount_path = dir.path().join("ownerfs");
    let reader_mount = dir.path().join("reader");
    fs::create_dir(&mount_path).unwrap();
    fs::create_dir(&reader_mount).unwrap();
    let (disk, roots, ownerfs) =
        ownerfs_fixture::ownerfs_fixture_with_cache(&dir.path().join("data"), native_eligible);
    let session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &mount_path).unwrap();
    let reader_session = afs::node::fuse::mount_ownerfs(ownerfs.clone(), &reader_mount).unwrap();
    let target = mount_path.join("agent1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("result"), b"BEFORE").unwrap();
    let read_path = reader_mount.join("agent1/result");
    assert_eq!(fs::read(&read_path).unwrap(), b"BEFORE");
    let id = root_id_from_name(OsStr::new("agent1")).unwrap();
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
    let active = manager.activate(&spec.identity).unwrap();
    assert_eq!(active.state, NativeState::NativeActive);
    assert_eq!(dir_id(&target), spec.source);
    // This fixture is the management side. Spawn the Agent only after the
    // verified export exists in its final (inherited) mount namespace.
    let child = Command::new("python3").arg("-I").arg("-c").arg(r#"
import json, os, pathlib, sys
expected = json.loads(sys.argv[1])
st = os.stat('.')
assert [st.st_dev, st.st_ino] == expected
with open('agent-created', 'wb') as f: f.write(b'agent native')
print(json.dumps({'namespace':os.readlink('/proc/self/ns/mnt'), 'device':st.st_dev, 'inode':st.st_ino}))
"#).arg(serde_json::to_string(&[spec.source.device, spec.source.inode]).unwrap())
        .current_dir(&target).output().unwrap();
    let results = (|| -> io::Result<Vec<serde_json::Value>> {
        let mut results = Vec::new();
        for content in [
            b"NEW-LONG-CONTENT".as_slice(),
            b"X".as_slice(),
            b"".as_slice(),
        ] {
            fs::write(target.join("result"), content)?; // writer closes before fresh reader open
            results.push(serde_json::json!({"expected":content, "observed":observed(&read_path)}));
        }
        fs::write(target.join("result"), b"OBJECT-A")?;
        let mut old = File::open(&read_path)?;
        let mut before = Vec::new();
        old.read_to_end(&mut before)?; // warm this old object before replacement
        fs::write(target.join("replacement"), b"OBJECT-B")?;
        fs::rename(target.join("replacement"), target.join("result"))?;
        results.push(serde_json::json!({"expected":b"OBJECT-B", "observed":observed(&read_path)}));
        old.seek(SeekFrom::Start(0))?;
        let mut retained = Vec::new();
        old.read_to_end(&mut retained)?;
        results.push(serde_json::json!({"old_before":before, "old_after":retained}));
        drop(old);
        // Warm directory identity and descendant lookup, move it natively,
        // then reopen strictly from the stable workspace root/current path.
        fs::create_dir(target.join("left"))?;
        fs::create_dir(target.join("right"))?;
        fs::create_dir(target.join("left/moving"))?;
        fs::write(target.join("left/moving/file"), b"moved child")?;
        let _ = fs::read(reader_mount.join("agent1/left/moving/file"));
        fs::rename(target.join("left/moving"), target.join("right/moving"))?;
        results.push(serde_json::json!({"expected":b"moved child", "observed":observed(&reader_mount.join("agent1/right/moving/file"))}));
        fs::remove_file(target.join("result"))?;
        results.push(serde_json::json!({"absent":observed(&read_path)}));
        Ok(results)
    })();
    manager.quiesce(&spec.identity).unwrap();
    assert_eq!(
        manager.detach(&spec.identity).unwrap().state,
        NativeState::Detached
    );
    let fallback = fs::read(target.join("agent-created"));
    drop(manager);
    drop(authority);
    for path in [&reader_mount, &mount_path] {
        assert!(Command::new("umount").arg(path).status().unwrap().success());
    }
    reader_session.join().unwrap();
    session.join().unwrap();
    println!(
        "native_cto native_eligible={native_eligible} source={:?} covered={:?} export={:?} agent_stdout={} results={results:?}",
        spec.source,
        spec.target,
        active.observed,
        String::from_utf8_lossy(&child.stdout)
    );
    assert!(
        child.status.success(),
        "Agent failed: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    let agent: serde_json::Value = serde_json::from_slice(&child.stdout).unwrap();
    assert_eq!(
        agent["namespace"],
        fs::read_link("/proc/self/ns/mnt")
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(fallback.unwrap(), b"agent native");
    let results = results.unwrap();
    for result in results {
        if let Some(expected) = result.get("expected") {
            assert_eq!(
                &result["observed"]["bytes"], expected,
                "close-to-open current path: {result}"
            );
            assert_eq!(
                result["observed"]["length"],
                expected.as_array().unwrap().len()
            );
        } else if result.get("old_before").is_some() {
            assert_eq!(result["old_before"], serde_json::json!(b"OBJECT-A"));
            assert_eq!(result["old_after"], serde_json::json!(b"OBJECT-A"));
        } else {
            assert_eq!(result["absent"]["errno"], libc::ENOENT);
        }
    }
}
