//! Disposable read-only kernel mechanism probe, NOT the production FUSE adapter.
//! One pinned moving directory and fixed in-process authority; no P2P/lifecycle claim.
use super::{RetainedReference, dir_id, ownerfs_fixture};
use afs::node::vfs::ownerfs::{
    native::*,
    root::{RootRight, root_id_from_name},
};
use afs::node::vfs::{Backend, ownerfs::OwnerFs, types::*};
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyEmpty, ReplyEntry, ReplyOpen,
    Request,
};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufRead, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, UNIX_EPOCH},
};

fn ctx(req: &Request<'_>) -> RequestContext {
    RequestContext {
        uid: req.uid(),
        gid: req.gid(),
        pid: req.pid(),
        umask: 0,
        supplementary_gids: Vec::new(),
    }
}
fn attr(ino: u64, a: &FileAttributes) -> FileAttr {
    FileAttr {
        ino,
        size: a.size,
        blocks: a.blocks,
        atime: a.atime,
        mtime: a.mtime,
        ctime: a.ctime,
        crtime: UNIX_EPOCH,
        kind: match a.kind {
            FileKind::Directory => FileType::Directory,
            FileKind::Regular => FileType::RegularFile,
            _ => panic!("outside read-only probe"),
        },
        perm: a.mode as u16,
        nlink: a.nlink,
        uid: a.uid,
        gid: a.gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}
fn err(e: afs_error::Error) -> i32 {
    afs::error::errno(&e)
}
fn fd_path(file: &File) -> io::Result<PathBuf> {
    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}
struct ProbeState {
    owner: Arc<OwnerFs>,
    moving: BackendInode,
    source_root: File,
    source_object: File,
    identity: DirectoryIdentity,
    covered: Mutex<Option<Arc<File>>>,
    last_path: Mutex<PathBuf>,
    helpers: Mutex<HashSet<u32>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    repair: bool,
    pin_attrs: bool,
}
impl ProbeState {
    fn needs_repair(&self, ino: u64, pid: u32) -> bool {
        self.repair
            && ino == self.moving.value
            && !self.helpers.lock().unwrap().contains(&pid)
            && fd_path(&self.source_object).unwrap() != *self.last_path.lock().unwrap()
    }
    fn reconcile(&self) -> io::Result<()> {
        let meta = self.source_object.metadata()?;
        if meta.nlink() == 0 {
            return Err(io::Error::from_raw_os_error(libc::ENOENT));
        }
        if (meta.dev(), meta.ino()) != (self.identity.device, self.identity.inode) {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
        let observed = fd_path(&self.source_object)?;
        let root = fd_path(&self.source_root)?;
        let relative = observed
            .strip_prefix(&root)
            .map_err(|_| io::Error::from_raw_os_error(libc::EXDEV))?;
        let covered = self.covered.lock().unwrap().as_ref().unwrap().clone();
        let task = fs::read_link("/proc/thread-self")?;
        let tid: u32 = task.file_name().unwrap().to_str().unwrap().parse().unwrap();
        {
            let mut helpers = self.helpers.lock().unwrap();
            helpers.insert(tid);
        }
        // This recursive lookup must execute off the FUSE receive thread.
        // The test has one actor and one moving object; the helper exemption
        // is probe-only and is not a production authorization/lifetime design.
        let result = fs::metadata(
            PathBuf::from(format!("/proc/self/fd/{}", covered.as_raw_fd())).join(relative),
        );
        {
            let mut helpers = self.helpers.lock().unwrap();
            helpers.remove(&tid);
        }
        let looked = result?;
        if !looked.is_dir()
            || looked.ino() != self.moving.value
            || fd_path(&self.source_object)? != observed
        {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
        println!(
            "directory_worker_reconciled source={:?} identity={:?} fuse_inode={} helper_tid={}",
            observed, self.identity, self.moving.value, tid
        );
        *self.last_path.lock().unwrap() = observed;
        Ok(())
    }
    fn reply_attr(&self, context: &RequestContext, ino: u64, reply: ReplyAttr) {
        // Diagnostic control only: the pinned deleted object remains readable
        // even though no backend path names it. This does not relocate its
        // kernel parent or authorize a production request.
        if self.pin_attrs && ino == self.moving.value {
            let m = self.source_object.metadata().unwrap();
            if m.nlink() == 0 {
                assert_eq!(
                    (m.dev(), m.ino()),
                    (self.identity.device, self.identity.inode)
                );
                reply.attr(
                    &Duration::ZERO,
                    &FileAttr {
                        ino,
                        size: m.len(),
                        blocks: m.blocks(),
                        atime: m.accessed().unwrap(),
                        mtime: m.modified().unwrap(),
                        ctime: UNIX_EPOCH
                            + Duration::new(
                                m.ctime().try_into().unwrap(),
                                m.ctime_nsec().try_into().unwrap(),
                            ),
                        crtime: UNIX_EPOCH,
                        kind: FileType::Directory,
                        perm: (m.mode() & 0o7777) as u16,
                        nlink: m.nlink().try_into().unwrap(),
                        uid: m.uid(),
                        gid: m.gid(),
                        rdev: 0,
                        blksize: m.blksize().try_into().unwrap(),
                        flags: 0,
                    },
                );
                return;
            }
        }
        match self
            .owner
            .getattr(context, BackendInode { value: ino }, None)
        {
            Ok(a) => reply.attr(&Duration::ZERO, &attr(ino, &a)),
            Err(e) => reply.error(err(e)),
        }
    }
}
struct ProbeFs(Arc<ProbeState>);
impl Filesystem for ProbeFs {
    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match self
            .0
            .owner
            .lookup(&ctx(req), BackendInode { value: parent }, name)
        {
            Ok(e) => reply.entry(&Duration::ZERO, &attr(e.inode.value, &e.attributes), 0),
            Err(e) => reply.error(err(e)),
        }
    }
    fn getattr(&mut self, req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        let context = ctx(req);
        if ino == self.0.moving.value && self.0.helpers.lock().unwrap().contains(&context.pid) {
            println!("directory_helper_header_tid={}", context.pid);
        }
        let pinned_deleted = self.0.pin_attrs
            && ino == self.0.moving.value
            && self.0.source_object.metadata().unwrap().nlink() == 0;
        if !pinned_deleted && self.0.needs_repair(ino, context.pid) {
            let state = self.0.clone();
            let worker = std::thread::spawn(move || match state.reconcile() {
                Ok(()) => state.reply_attr(&context, ino, reply),
                Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
            });
            self.0.workers.lock().unwrap().push(worker);
        } else {
            self.0.reply_attr(&context, ino, reply);
        }
    }
    fn opendir(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(ino, 0);
    }
    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }
    fn open(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        flags: i32,
        _open_flags: u32,
        reply: ReplyOpen,
    ) {
        match self
            .0
            .owner
            .open(&ctx(req), BackendInode { value: ino }, flags)
        {
            Ok(h) => reply.opened(h.0, fuser::consts::FOPEN_DIRECT_IO),
            Err(e) => reply.error(err(e)),
        }
    }
    fn read(
        &mut self,
        req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _owner: Option<u64>,
        reply: ReplyData,
    ) {
        let mut data = vec![0_u8; size as usize];
        match self
            .0
            .owner
            .read(&ctx(req), FileHandle(fh), offset as u64, &mut data)
        {
            Ok(n) => reply.data(&data[..n]),
            Err(e) => reply.error(err(e)),
        }
    }
    fn flush(&mut self, _req: &Request<'_>, _ino: u64, _fh: u64, _owner: u64, reply: ReplyEmpty) {
        reply.ok();
    }
    fn release(
        &mut self,
        req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.0.owner.release(&ctx(req), FileHandle(fh)) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(err(e)),
        }
    }
}
pub fn run(deleted: bool, moved: bool) {
    assert_eq!(
        std::env::var("AFS_NATIVE_PRIVATE_NAMESPACE").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let (disk, roots, owner) = ownerfs_fixture::ownerfs_fixture(&temp.path().join("data"));
    let context = RequestContext {
        uid: temp.path().metadata().unwrap().uid(),
        gid: temp.path().metadata().unwrap().gid(),
        pid: std::process::id(),
        umask: 0,
        supplementary_gids: Vec::new(),
    };
    let workspace = owner
        .mkdir(
            &context,
            BackendInode { value: 1 },
            OsStr::new("agent1"),
            0o755,
        )
        .unwrap();
    let left = owner
        .mkdir(&context, workspace.inode, OsStr::new("left"), 0o755)
        .unwrap();
    let right = owner
        .mkdir(&context, workspace.inode, OsStr::new("right"), 0o755)
        .unwrap();
    let moving = owner
        .mkdir(&context, left.inode, OsStr::new("moving"), 0o755)
        .unwrap();
    for (parent, name, data) in [
        (left.inode, "parent-id", b"LEFT".as_slice()),
        (right.inode, "parent-id", b"RIGHT".as_slice()),
        (moving.inode, "data", b"same directory".as_slice()),
    ] {
        let file = owner
            .create(&context, parent, OsStr::new(name), 0o644, libc::O_RDWR)
            .unwrap();
        owner.write(&context, file.handle, 0, data).unwrap();
        owner.release(&context, file.handle).unwrap();
    }
    let id = root_id_from_name(OsStr::new("agent1")).unwrap();
    let authority = roots.enter_root(&id, RootRight::Write).unwrap();
    let source = disk.root_path().join(authority.data_dir().as_path());
    let object = source.join("left/moving");
    let state = Arc::new(ProbeState {
        owner: owner.clone(),
        moving: moving.inode,
        source_root: File::open(&source).unwrap(),
        source_object: File::open(&object).unwrap(),
        identity: dir_id(&object),
        covered: Mutex::new(None),
        last_path: Mutex::new(object.clone()),
        helpers: Mutex::new(HashSet::new()),
        workers: Mutex::new(Vec::new()),
        repair: std::env::var("AFS_NATIVE_REFERENCE_REPAIR").as_deref() == Ok("1"),
        pin_attrs: std::env::var("AFS_NATIVE_PINNED_DIRECTORY_ATTR").as_deref() == Ok("1"),
    });
    let mount = temp.path().join("ownerfs");
    fs::create_dir(&mount).unwrap();
    let session = fuser::spawn_mount2(
        ProbeFs(state.clone()),
        &mount,
        &[
            fuser::MountOption::FSName("afs-native-directory-probe".into()),
            fuser::MountOption::DefaultPermissions,
        ],
    )
    .unwrap();
    let target = mount.join("agent1");
    *state.covered.lock().unwrap() = Some(Arc::new(File::open(&target).unwrap()));
    let script = r#"
import os, sys, json, stat
os.chdir(sys.argv[1])
legacy = os.open('.', os.O_RDONLY | os.O_DIRECTORY)
native = os.open(sys.argv[2], os.O_RDONLY | os.O_DIRECTORY)
print(json.dumps({'ready':True,'pid':os.getpid()}), flush=True)
assert sys.stdin.readline().strip() == 'renamed'
def read_at(directory, name):
    try:
        fd = os.open(name, os.O_RDONLY, dir_fd=directory)
        try: return {'data':os.read(fd,1024).decode('ascii')}
        finally: os.close(fd)
    except OSError as e: return {'errno':e.errno}
def attrs(directory):
    try:
        a = os.fstat(directory)
        return {'nlink':a.st_nlink, 'mode':stat.S_IMODE(a.st_mode), 'is_dir':stat.S_ISDIR(a.st_mode)}
    except OSError as e: return {'errno':e.errno}
# Parent-only operation comes first, with no caller new-path lookup.
def observation():
    return {'legacy_parent':read_at(legacy,'../parent-id'),'native_parent':read_at(native,'../parent-id'),'cwd_parent':read_at(None,'../parent-id'),'legacy_data':read_at(legacy,'data'),'native_data':read_at(native,'data'),'legacy_attrs':attrs(legacy),'native_attrs':attrs(native)}
print(json.dumps(observation()), flush=True)
if sys.argv[3] == 'named':
    assert sys.stdin.readline().strip() == 'renamed-again'
    print(json.dumps(observation()), flush=True)
os.close(native)
os.close(legacy)
"#;
    let mut actor = RetainedReference(
        std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(target.join("left/moving"))
            .arg(&object)
            .arg(if deleted { "deleted" } else { "named" })
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = io::BufReader::new(actor.0.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["ready"], true);
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
            File::open(&mount).unwrap(),
            OsStr::new("agent1"),
            MountPolicy::default(),
        )
        .unwrap();
    let mounted = backend.bind(&spec).unwrap();
    if moved {
        fs::rename(target.join("left/moving"), target.join("right/moving")).unwrap();
    }
    if deleted {
        let deleted_path = target.join(if moved { "right/moving" } else { "left/moving" });
        fs::remove_file(deleted_path.join("data")).unwrap();
        fs::remove_dir(deleted_path).unwrap();
    } else {
        // Reusing the old name must not redirect the retained directory reference.
        fs::create_dir(target.join("left/moving")).unwrap();
        fs::write(target.join("left/moving/data"), b"replacement directory").unwrap();
    }
    writeln!(actor.0.stdin.as_mut().unwrap(), "renamed").unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let observed: serde_json::Value = serde_json::from_str(&line).unwrap();
    let second = if deleted {
        serde_json::Value::Null
    } else {
        // This fresh-name control runs AFTER the first user observations, so it
        // cannot conceal a missing automatic repair of the initial old reference.
        let covered = state.covered.lock().unwrap().as_ref().unwrap().clone();
        let replacement_path = PathBuf::from(format!("/proc/self/fd/{}", covered.as_raw_fd()))
            .join("left/moving/data");
        assert_eq!(
            fs::read(&replacement_path).unwrap(),
            b"replacement directory"
        );
        drop(covered);
        fs::remove_file(target.join("left/moving/data")).unwrap();
        fs::remove_dir(target.join("left/moving")).unwrap();
        fs::rename(target.join("right/moving"), target.join("left/moving")).unwrap();
        writeln!(actor.0.stdin.as_mut().unwrap(), "renamed-again").unwrap();
        line.clear();
        output.read_line(&mut line).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    };
    assert!(actor.0.wait().unwrap().success());
    for worker in state.workers.lock().unwrap().drain(..) {
        worker.join().unwrap();
    }
    state.covered.lock().unwrap().take();
    backend.unmount(&spec, &mounted).unwrap();
    backend.release_prepared(&spec).unwrap();
    drop(backend);
    drop(authority);
    assert!(
        std::process::Command::new("umount")
            .arg(&mount)
            .status()
            .unwrap()
            .success()
    );
    drop(session);
    println!(
        "directory_reference_probe deleted={deleted} moved={moved} repair={} pin_attrs={} actor={ready} observed={observed} second={second}",
        state.repair, state.pin_attrs
    );
    assert_eq!(
        observed["native_parent"]["data"],
        if moved { "RIGHT" } else { "LEFT" }
    );
    if deleted {
        assert_eq!(observed["native_data"]["errno"], libc::ENOENT);
        assert_eq!(observed["native_attrs"]["nlink"], 0);
        assert_eq!(observed["native_attrs"]["is_dir"], true);
    } else {
        assert_eq!(observed["native_data"]["data"], "same directory");
    }
    assert_eq!(
        observed["legacy_parent"], observed["native_parent"],
        "automatic first parent traversal"
    );
    assert_eq!(
        observed["cwd_parent"], observed["native_parent"],
        "automatic cwd parent traversal"
    );
    assert_eq!(
        observed["legacy_data"], observed["native_data"],
        "automatic old-directory child traversal"
    );
    assert_eq!(
        observed["legacy_attrs"], observed["native_attrs"],
        "retained directory fstat"
    );
    if deleted {
        return;
    }
    assert_eq!(second["native_parent"]["data"], "LEFT");
    assert_eq!(second["native_data"]["data"], "same directory");
    assert_eq!(
        second["legacy_parent"], second["native_parent"],
        "automatic repeated parent traversal"
    );
    assert_eq!(
        second["cwd_parent"], second["native_parent"],
        "automatic repeated cwd traversal"
    );
    assert_eq!(
        second["legacy_data"], second["native_data"],
        "automatic repeated child traversal"
    );
}
