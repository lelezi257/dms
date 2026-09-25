use std::{io, sync::Arc, time::Duration};

use afs::node::{fuse, vfs::Vfs};

#[test]
#[ignore = "requires Linux /dev/fuse and fusermount3"]
fn fuse_mount_lists_namespaces_and_dispatches_create_as_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let mount = temp.path().join("mnt");
    std::fs::create_dir(&mount).unwrap();

    let vfs = Arc::new(Vfs::new(true, true, afs_metrics::registry()).unwrap());
    let session = fuse::mount(vfs, &mount).unwrap();
    wait_until_mounted(&mount).unwrap();

    assert_eq!(namespace_entries(&mount), vec!["blobfs", "ownerfs"]);

    let error = std::fs::File::create(mount.join("ownerfs").join("hello.txt")).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENOSYS));

    drop(session);
    let _ = std::process::Command::new("fusermount3")
        .arg("-u")
        .arg(&mount)
        .status();
}

#[test]
#[ignore = "requires Linux /dev/fuse and fusermount3"]
fn fuse_mount_hides_runtime_disabled_backend() {
    let temp = tempfile::tempdir().unwrap();
    let mount = temp.path().join("mnt");
    std::fs::create_dir(&mount).unwrap();

    let vfs = Arc::new(Vfs::new(true, false, afs_metrics::registry()).unwrap());
    let session = fuse::mount(vfs, &mount).unwrap();
    wait_until_mounted(&mount).unwrap();

    assert_eq!(namespace_entries(&mount), vec!["ownerfs"]);
    let error = std::fs::metadata(mount.join("blobfs")).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENOENT));

    drop(session);
    let _ = std::process::Command::new("fusermount3")
        .arg("-u")
        .arg(&mount)
        .status();
}

#[test]
#[ignore = "requires Linux /dev/fuse and fusermount3"]
fn fuse_mount_rejects_existing_live_mount() {
    let temp = tempfile::tempdir().unwrap();
    let mount = temp.path().join("mnt");
    std::fs::create_dir(&mount).unwrap();

    let vfs = Arc::new(Vfs::new(true, true, afs_metrics::registry()).unwrap());
    let session = fuse::mount(vfs, &mount).unwrap();
    wait_until_mounted(&mount).unwrap();

    let second = Arc::new(Vfs::new(true, false, afs_metrics::registry()).unwrap());
    let error = fuse::mount(second, &mount).unwrap_err();
    assert_eq!(error.kind(), afs_error::ErrorKind::Conflict);

    assert_eq!(namespace_entries(&mount), vec!["blobfs", "ownerfs"]);

    drop(session);
    let _ = std::process::Command::new("fusermount3")
        .arg("-u")
        .arg(&mount)
        .status();
}

fn namespace_entries(mount: &std::path::Path) -> Vec<String> {
    let mut entries = std::fs::read_dir(mount)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn wait_until_mounted(mount: &std::path::Path) -> io::Result<()> {
    let start = std::time::Instant::now();
    loop {
        match std::fs::read_dir(mount) {
            Ok(_) => return Ok(()),
            Err(error) if start.elapsed() < Duration::from_secs(5) => {
                if !matches!(
                    error.raw_os_error(),
                    Some(libc::ENOTCONN | libc::ENOENT | libc::EAGAIN)
                ) {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}
