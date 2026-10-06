// Included inside private native_home_tests only. Run one exact ignored test
// inside an owned private Linux mount namespace; this is not a native manager.
fn n2a_mount_command(program: &str, arguments: &[&std::path::Path]) {
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()
        .unwrap();
    println!(
        "N2A_MOUNT_COMMAND program={program:?} arguments={arguments:?} status={:?} stdout={:?} stderr={:?}",
        output.status.code(),
        output.stdout,
        output.stderr
    );
    assert!(output.status.success(), "owned normal mount command failed");
}

struct N2aOwnedCover {
    target: std::path::PathBuf,
    mounted: bool,
}

impl N2aOwnedCover {
    fn unmount(&mut self) {
        n2a_mount_command("umount", &[self.target.as_path()]);
        self.mounted = false;
    }
}

impl Drop for N2aOwnedCover {
    fn drop(&mut self) {
        if self.mounted {
            // Keep all owned data on failure. No lazy unmount and no recursive
            // deletion are permitted while a cover could remain mounted.
            let result = std::process::Command::new("umount")
                .arg(&self.target)
                .output();
            println!("N2A_FAILURE_NORMAL_UNMOUNT {result:?}");
        }
    }
}

#[test]
#[ignore = "requires exact Linux ARM64 private namespace, root and /dev/fuse"]
fn native_home_real_covered_root_lifecycle() {
    assert_eq!(std::env::consts::OS, "linux");
    assert_eq!(std::env::consts::ARCH, "aarch64");
    let uid = std::process::Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    assert_eq!(uid.stdout, b"0\n");

    let (temp, fs, ctx, disk) = fixture(true);
    // Retain the directory even if an assertion or normal unmount fails.
    let retained = temp.keep();
    let root = mkdir_root(&fs, &ctx, "native");
    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    let source = retained.join(authority.data_dir().as_path());
    let file_source = source.join("file");
    fs::write(&file_source, vec![b'N'; 4096]).unwrap();
    let file = Backend::lookup(&fs, &ctx, root.inode, OsStr::new("file")).unwrap();
    let mount = retained.join("n2a-mount");
    fs::create_dir(&mount).unwrap();
    let target = mount.join("native");
    let ownerfs = Arc::new(fs);
    let namespace = fs::metadata("/proc/self/ns/mnt").unwrap();
    println!(
        "N2A_FUSE_OWNED root={retained:?} namespace_dev={} namespace_ino={}",
        namespace.dev(),
        namespace.ino()
    );
    println!(
        "N2A_MOUNTINFO_BEFORE {}",
        fs::read_to_string("/proc/self/mountinfo").unwrap()
    );
    let session = crate::node::fuse::mount_ownerfs(ownerfs.clone(), &mount).unwrap();
    assert!(fs::metadata(&target).unwrap().is_dir());
    ownerfs.with_fuse_cache_policy(file.inode, |ttl, private| {
        assert_eq!(ttl, Duration::ZERO);
        assert!(!private);
    });
    let mmap = std::process::Command::new("python3")
        .arg("-c")
        .arg("import mmap,os,sys; f=open(sys.argv[1],'r+b',buffering=0); m=mmap.mmap(f.fileno(),4096); m[:4]=b'n2a!'; m.flush(); os.fsync(f.fileno()); m.close(); f.close(); assert open(sys.argv[1],'rb').read()==b'n2a!'+b'N'*4092")
        .arg(target.join("file"))
        .output()
        .unwrap();
    println!(
        "N2A_ELIGIBLE_FUSE_MMAP_SMOKE_CURRENT_OPEN_POLICY status={:?} stdout={:?} stderr={:?}",
        mmap.status.code(),
        mmap.stdout,
        mmap.stderr
    );
    assert!(mmap.status.success());
    assert_eq!(
        fs::read(&file_source).unwrap(),
        [b"n2a!".as_slice(), &vec![b'N'; 4092]].concat()
    );

    n2a_mount_command("mount", &[std::path::Path::new("--bind"), &source, &target]);
    let mut cover = N2aOwnedCover {
        target: target.clone(),
        mounted: true,
    };
    let native = fs::metadata(&target).unwrap();
    let physical = fs::metadata(&source).unwrap();
    assert_eq!(
        (native.dev(), native.ino()),
        (physical.dev(), physical.ino())
    );
    println!(
        "N2A_MOUNTINFO_COVERED {}",
        fs::read_to_string("/proc/self/mountinfo").unwrap()
    );

    ownerfs
        .require_local()
        .unwrap()
        .roots
        .revoke_root(authority.root_id());
    assert!(authority.verify_current(&ownerfs).is_err());
    assert!(Backend::lookup(ownerfs.as_ref(), &ctx, root.inode, OsStr::new("file")).is_err());
    assert!(Backend::open(ownerfs.as_ref(), &ctx, file.inode, libc::O_RDONLY).is_err());
    assert_eq!(
        Backend::lookup(
            ownerfs.as_ref(),
            &ctx,
            ownerfs.root_inode(),
            OsStr::new("native")
        )
        .unwrap()
        .inode,
        root.inode
    );
    Backend::getattr(ownerfs.as_ref(), &ctx, root.inode, None).unwrap();

    // The anchor remains available while the real covered root is normally
    // uncovered. No final runtime clone or production READY is involved.
    cover.unmount();
    assert!(fs::metadata(&target).unwrap().is_dir());
    assert!(fs::read(target.join("file")).is_err());
    drop(authority);
    assert!(
        Backend::lookup(
            ownerfs.as_ref(),
            &ctx,
            ownerfs.root_inode(),
            OsStr::new("native")
        )
        .is_err()
    );
    assert!(Backend::getattr(ownerfs.as_ref(), &ctx, root.inode, None).is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fs::metadata(&target).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "kernel root metadata persisted after backend anchor drop; mountinfo={}",
            fs::read_to_string("/proc/self/mountinfo").unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Normal outer FUSE unmount/shutdown follows normal bind-cover unmount.
    session.join().unwrap();
    let after = fs::read_to_string("/proc/self/mountinfo").unwrap();
    println!("N2A_MOUNTINFO_AFTER {after}");
    assert!(!after.lines().any(|line| {
        line.split_whitespace()
            .nth(4)
            .is_some_and(|path| path.starts_with(retained.to_str().unwrap()))
    }));
    drop(cover);
    drop(ownerfs);
    drop(disk);
    fs::remove_dir_all(&retained).unwrap();
    println!("N2A_COVERED_ROOT_LIFECYCLE PASS normal_unmount=true ready=false runtime_clone=false");
}
