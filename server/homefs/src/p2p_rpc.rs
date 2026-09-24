//! Trusted-node file RPC for the Agent-home preview.
//!
//! The transport is intentionally small: authenticated TCP peers forward
//! bounded file operations to the home node's ordinary file tree. It does not
//! implement replay recovery, replication, or cross-node failover; a missing
//! home must remain a clear remote failure for this preview.
use crate::home_fuse::{
    PrivateAttrCache, checked_mkdir, checked_open, checked_read_dir, checked_rename,
    checked_unlink, open_parent_dir,
};
use fuser::{FileAttr, FileType, Notifier};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::{
        fd::{AsRawFd, IntoRawFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_IO_BYTES: usize = MAX_FRAME - 128;
const OPEN_PREFETCH_LIMIT: usize = 4096;
const WIRE_VERSION: u32 = 3;
const AUTH: u8 = 0;
const GETATTR: u8 = 1;
const MKDIR: u8 = 2;
const CREATE: u8 = 3;
const OPEN: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const FSYNC: u8 = 7;
const CLOSE: u8 = 8;
const RENAME: u8 = 9;
const UNLINK: u8 = 10;
const RMDIR: u8 = 11;
const READDIR: u8 = 12;
const OPENDIR: u8 = 13;
const SETLEN: u8 = 14;
const SETATTR: u8 = 15;
const CLOSE_NO_REPLY: u8 = 16;

const SETATTR_MODE: u32 = 1 << 0;
const SETATTR_UID: u32 = 1 << 1;
const SETATTR_GID: u32 = 1 << 2;
const SETATTR_ATIME: u32 = 1 << 3;
const SETATTR_MTIME: u32 = 1 << 4;

const TIME_NOW: u8 = 0;
const TIME_AT: u8 = 1;

fn errno(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or_else(|| match error.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset => libc::ECONNRESET,
        io::ErrorKind::TimedOut => libc::ETIMEDOUT,
        _ => libc::EIO,
    })
}
fn invalid() -> io::Error {
    io::Error::from(io::ErrorKind::InvalidData)
}

pub struct Writer(Vec<u8>);
impl Writer {
    pub fn new(op: u8) -> Self {
        Self(vec![op])
    }
    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub fn u32(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.0.extend(value.to_be_bytes());
    }
    pub fn i64(&mut self, value: i64) {
        self.0.extend(value.to_be_bytes());
    }
    pub fn bytes(&mut self, value: &[u8]) {
        self.u32(value.len() as u32);
        self.0.extend(value);
    }
    pub fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self.offset.checked_add(count).ok_or_else(invalid)?;
        let result = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(result)
    }
    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_be_bytes(bytes))
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(bytes))
    }
    pub fn i64(&mut self) -> io::Result<i64> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(i64::from_be_bytes(bytes))
    }
    pub fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }
    pub fn string(&mut self) -> io::Result<&'a str> {
        std::str::from_utf8(self.bytes()?).map_err(|_| invalid())
    }
    pub fn done(&self) -> io::Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}

fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(invalid());
    }
    if bytes.len() <= 8192 {
        let mut frame = Vec::with_capacity(4 + bytes.len());
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(bytes);
        stream.write_all(&frame)
    } else {
        stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
        stream.write_all(bytes)
    }
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: i64,
    pub atime_ns: u32,
    pub mtime: i64,
    pub mtime_ns: u32,
    pub ctime: i64,
    pub ctime_ns: u32,
    pub kind: u8,
    pub perm: u16,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
}
impl Attr {
    fn from_metadata(meta: &fs::Metadata) -> Result<Self, i32> {
        let kind = if meta.file_type().is_symlink() {
            return Err(libc::ELOOP);
        } else if meta.is_dir() {
            1
        } else if meta.is_file() {
            2
        } else {
            return Err(libc::EOPNOTSUPP);
        };
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            blocks: meta.blocks(),
            atime: meta.atime(),
            atime_ns: meta.atime_nsec() as u32,
            mtime: meta.mtime(),
            mtime_ns: meta.mtime_nsec() as u32,
            ctime: meta.ctime(),
            ctime_ns: meta.ctime_nsec() as u32,
            kind,
            perm: meta.mode() as u16 & 0o7777,
            nlink: meta.nlink() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            rdev: meta.rdev() as u32,
            blksize: meta.blksize() as u32,
        })
    }
    fn write(&self, out: &mut Writer) {
        out.u64(self.dev);
        out.u64(self.ino);
        out.u64(self.size);
        out.u64(self.blocks);
        out.i64(self.atime);
        out.u32(self.atime_ns);
        out.i64(self.mtime);
        out.u32(self.mtime_ns);
        out.i64(self.ctime);
        out.u32(self.ctime_ns);
        out.u8(self.kind);
        out.u32(self.perm as u32);
        out.u32(self.nlink);
        out.u32(self.uid);
        out.u32(self.gid);
        out.u32(self.rdev);
        out.u32(self.blksize);
    }
    fn read(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            dev: input.u64()?,
            ino: input.u64()?,
            size: input.u64()?,
            blocks: input.u64()?,
            atime: input.i64()?,
            atime_ns: input.u32()?,
            mtime: input.i64()?,
            mtime_ns: input.u32()?,
            ctime: input.i64()?,
            ctime_ns: input.u32()?,
            kind: input.u8()?,
            perm: input.u32()? as u16,
            nlink: input.u32()?,
            uid: input.u32()?,
            gid: input.u32()?,
            rdev: input.u32()?,
            blksize: input.u32()?,
        })
    }
    pub fn file_attr(&self, ino: u64) -> FileAttr {
        let stamp = |sec: i64, ns: u32| {
            if sec >= 0 {
                UNIX_EPOCH + Duration::new(sec as u64, ns)
            } else {
                SystemTime::UNIX_EPOCH
            }
        };
        FileAttr {
            ino,
            size: self.size,
            blocks: self.blocks,
            atime: stamp(self.atime, self.atime_ns),
            mtime: stamp(self.mtime, self.mtime_ns),
            ctime: stamp(self.ctime, self.ctime_ns),
            crtime: UNIX_EPOCH,
            kind: if self.kind == 1 {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: self.perm,
            nlink: self.nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: self.rdev,
            blksize: self.blksize,
            flags: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeSpec {
    Now,
    At(i64, u32),
}

impl TimeSpec {
    fn write(self, out: &mut Writer) {
        match self {
            Self::Now => out.u8(TIME_NOW),
            Self::At(sec, nsec) => {
                out.u8(TIME_AT);
                out.i64(sec);
                out.u32(nsec);
            }
        }
    }

    fn read(input: &mut Reader<'_>) -> io::Result<Self> {
        match input.u8()? {
            TIME_NOW => Ok(Self::Now),
            TIME_AT => Ok(Self::At(input.i64()?, input.u32()?)),
            _ => Err(invalid()),
        }
    }

    fn timespec(value: Option<Self>) -> Result<libc::timespec, i32> {
        match value {
            Some(Self::Now) => Ok(libc::timespec {
                tv_sec: 0,
                tv_nsec: libc::UTIME_NOW as _,
            }),
            Some(Self::At(sec, nsec)) => {
                if nsec >= 1_000_000_000 {
                    return Err(libc::EINVAL);
                }
                Ok(libc::timespec {
                    tv_sec: sec as _,
                    tv_nsec: nsec as _,
                })
            }
            None => Ok(libc::timespec {
                tv_sec: 0,
                tv_nsec: libc::UTIME_OMIT as _,
            }),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetAttrSpec {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub atime: Option<TimeSpec>,
    pub mtime: Option<TimeSpec>,
}

impl SetAttrSpec {
    fn write(&self, out: &mut Writer) {
        let mut mask = 0;
        if self.mode.is_some() {
            mask |= SETATTR_MODE;
        }
        if self.uid.is_some() {
            mask |= SETATTR_UID;
        }
        if self.gid.is_some() {
            mask |= SETATTR_GID;
        }
        if self.atime.is_some() {
            mask |= SETATTR_ATIME;
        }
        if self.mtime.is_some() {
            mask |= SETATTR_MTIME;
        }
        out.u32(mask);
        if let Some(mode) = self.mode {
            out.u32(mode);
        }
        if let Some(uid) = self.uid {
            out.u32(uid);
        }
        if let Some(gid) = self.gid {
            out.u32(gid);
        }
        if let Some(atime) = self.atime {
            atime.write(out);
        }
        if let Some(mtime) = self.mtime {
            mtime.write(out);
        }
    }

    fn read(input: &mut Reader<'_>) -> io::Result<Self> {
        let mask = input.u32()?;
        if mask & !(SETATTR_MODE | SETATTR_UID | SETATTR_GID | SETATTR_ATIME | SETATTR_MTIME) != 0 {
            return Err(invalid());
        }
        Ok(Self {
            mode: if mask & SETATTR_MODE != 0 {
                Some(input.u32()?)
            } else {
                None
            },
            uid: if mask & SETATTR_UID != 0 {
                Some(input.u32()?)
            } else {
                None
            },
            gid: if mask & SETATTR_GID != 0 {
                Some(input.u32()?)
            } else {
                None
            },
            atime: if mask & SETATTR_ATIME != 0 {
                Some(TimeSpec::read(input)?)
            } else {
                None
            },
            mtime: if mask & SETATTR_MTIME != 0 {
                Some(TimeSpec::read(input)?)
            } else {
                None
            },
        })
    }
}

pub struct Client(TcpStream);
impl Client {
    #[allow(dead_code)]
    pub fn connect(address: &str) -> io::Result<Self> {
        Self::connect_authenticated(address, "")
    }
    pub fn connect_authenticated(address: &str, token: &str) -> io::Result<Self> {
        let address: SocketAddr = address.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "P2P endpoint must be an IP:port",
            )
        })?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut auth = Writer::new(AUTH);
        auth.u32(WIRE_VERSION);
        auth.bytes(token.as_bytes());
        write_frame(&mut stream, &auth.finish())?;
        let response = read_frame(&mut stream)?;
        let mut input = Reader::new(&response);
        let status = input.u32()? as i32;
        input.done()?;
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        Ok(Self(stream))
    }
    fn call(&mut self, request: Writer) -> Result<Vec<u8>, i32> {
        write_frame(&mut self.0, &request.finish()).map_err(errno)?;
        let response = read_frame(&mut self.0).map_err(errno)?;
        let mut input = Reader::new(&response);
        let status = input.u32().map_err(errno)? as i32;
        if status != 0 {
            return Err(status);
        }
        Ok(response[4..].to_vec())
    }
    pub fn getattr(&mut self, path: &str) -> Result<Attr, i32> {
        let mut out = Writer::new(GETATTR);
        out.string(path);
        let bytes = self.call(out)?;
        Attr::read(&mut Reader::new(&bytes)).map_err(errno)
    }
    pub fn mkdir(&mut self, path: &str, mode: u32) -> Result<Attr, i32> {
        let mut out = Writer::new(MKDIR);
        out.string(path);
        out.u32(mode);
        let bytes = self.call(out)?;
        Attr::read(&mut Reader::new(&bytes)).map_err(errno)
    }
    pub fn create(&mut self, path: &str, flags: i32, mode: u32) -> Result<(u64, Attr), i32> {
        let mut out = Writer::new(CREATE);
        out.string(path);
        out.u32(flags as u32);
        out.u32(mode);
        let bytes = self.call(out)?;
        let mut input = Reader::new(&bytes);
        let handle = input.u64().map_err(errno)?;
        let attr = Attr::read(&mut input).map_err(errno)?;
        Ok((handle, attr))
    }
    pub fn open(
        &mut self,
        path: &str,
        flags: i32,
        directory: bool,
    ) -> Result<(u64, Attr, Option<Vec<u8>>), i32> {
        let mut out = Writer::new(if directory { OPENDIR } else { OPEN });
        out.string(path);
        out.u32(flags as u32);
        let bytes = self.call(out)?;
        let mut input = Reader::new(&bytes);
        let handle = input.u64().map_err(errno)?;
        let attr = Attr::read(&mut input).map_err(errno)?;
        let prefetched = match input.u8().map_err(errno)? {
            0 => None,
            1 => Some(input.bytes().map_err(errno)?.to_vec()),
            _ => return Err(libc::EIO),
        };
        input.done().map_err(errno)?;
        Ok((handle, attr, prefetched))
    }
    pub fn read(&mut self, handle: u64, offset: u64, size: u32) -> Result<Vec<u8>, i32> {
        if offset > i64::MAX as u64 || size as usize > MAX_IO_BYTES {
            return Err(libc::EINVAL);
        }
        let mut out = Writer::new(READ);
        out.u64(handle);
        out.u64(offset);
        out.u32(size);
        let bytes = self.call(out)?;
        Reader::new(&bytes)
            .bytes()
            .map(|v| v.to_vec())
            .map_err(errno)
    }
    pub fn write(&mut self, handle: u64, offset: u64, bytes: &[u8]) -> Result<(u32, Attr), i32> {
        if offset > i64::MAX as u64 || bytes.len() > MAX_IO_BYTES {
            return Err(libc::EINVAL);
        }
        let mut out = Writer::new(WRITE);
        out.u64(handle);
        out.u64(offset);
        out.bytes(bytes);
        let bytes = self.call(out)?;
        let mut input = Reader::new(&bytes);
        let written = input.u32().map_err(errno)?;
        let attr = Attr::read(&mut input).map_err(errno)?;
        input.done().map_err(errno)?;
        Ok((written, attr))
    }
    pub fn fsync(&mut self, handle: u64, datasync: bool) -> Result<(), i32> {
        let mut out = Writer::new(FSYNC);
        out.u64(handle);
        out.u8(u8::from(datasync));
        self.call(out).map(|_| ())
    }
    pub fn close(&mut self, handle: u64) -> Result<(), i32> {
        let mut out = Writer::new(CLOSE);
        out.u64(handle);
        self.call(out).map(|_| ())
    }
    pub fn close_no_reply(&mut self, handle: u64) -> Result<(), i32> {
        let mut out = Writer::new(CLOSE_NO_REPLY);
        out.u64(handle);
        write_frame(&mut self.0, &out.finish()).map_err(errno)
    }
    pub fn rename(&mut self, from: &str, to: &str) -> Result<Attr, i32> {
        let mut out = Writer::new(RENAME);
        out.string(from);
        out.string(to);
        let bytes = self.call(out)?;
        Attr::read(&mut Reader::new(&bytes)).map_err(errno)
    }
    pub fn unlink(&mut self, path: &str, directory: bool) -> Result<(), i32> {
        let mut out = Writer::new(if directory { RMDIR } else { UNLINK });
        out.string(path);
        self.call(out).map(|_| ())
    }
    pub fn readdir(&mut self, path: &str) -> Result<Vec<(String, FileType)>, i32> {
        let mut out = Writer::new(READDIR);
        out.string(path);
        let bytes = self.call(out)?;
        let mut input = Reader::new(&bytes);
        let count = input.u32().map_err(errno)?;
        let mut rows = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let name = input.string().map_err(errno)?.to_owned();
            let kind = if input.u8().map_err(errno)? == 1 {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            rows.push((name, kind));
        }
        input.done().map_err(errno)?;
        Ok(rows)
    }
    pub fn set_len(&mut self, path: &str, size: u64) -> Result<Attr, i32> {
        let mut out = Writer::new(SETLEN);
        out.string(path);
        out.u64(size);
        let bytes = self.call(out)?;
        Attr::read(&mut Reader::new(&bytes)).map_err(errno)
    }
    pub fn setattr(&mut self, path: &str, spec: &SetAttrSpec) -> Result<Attr, i32> {
        let mut out = Writer::new(SETATTR);
        out.string(path);
        spec.write(&mut out);
        let bytes = self.call(out)?;
        Attr::read(&mut Reader::new(&bytes)).map_err(errno)
    }
}

#[allow(dead_code)]
pub fn serve(address: &str, root: &Path) -> io::Result<()> {
    serve_with_token(address, root, "")
}

pub fn serve_with_token(address: &str, root: &Path, token: &str) -> io::Result<()> {
    let listener = TcpListener::bind(address)?;
    serve_listener(listener, root.to_path_buf(), token.to_owned())
}

pub fn serve_listener(listener: TcpListener, root: PathBuf, token: String) -> io::Result<()> {
    serve_listener_inner(listener, root, token, None)
}

pub fn serve_listener_with_private_cache(
    listener: TcpListener,
    root: PathBuf,
    token: String,
    cache: Arc<PrivateAttrCache>,
    notifier: Notifier,
) -> io::Result<()> {
    serve_listener_inner(listener, root, token, Some((cache, notifier)))
}

fn serve_listener_inner(
    listener: TcpListener,
    root: PathBuf,
    token: String,
    private_cache: Option<(Arc<PrivateAttrCache>, Notifier)>,
) -> io::Result<()> {
    let root = root.canonicalize()?;
    eprintln!(
        "P2P file server listening on {}, root={}",
        listener.local_addr()?,
        root.display()
    );
    for stream in listener.incoming() {
        let mut stream = stream?;
        let root = root.clone();
        let token = token.clone();
        let private_cache = private_cache.clone();
        std::thread::spawn(move || {
            let _ = stream.set_nodelay(true);
            let auth_result = read_frame(&mut stream)
                .map_err(errno)
                .and_then(|request| authenticate(&request, &token));
            if write_status(&mut stream, auth_result).is_err() || auth_result.is_err() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
            let mut state = Session {
                root,
                handles: HashMap::new(),
                next_handle: 1,
            };
            while let Ok(request) = read_frame(&mut stream) {
                let result = match &private_cache {
                    Some((cache, notifier)) => enter_shared_request(&request, cache, notifier)
                        .and_then(|()| state.handle(&request)),
                    None => state.handle(&request),
                };
                if request.first() == Some(&CLOSE_NO_REPLY) {
                    continue;
                }
                let mut response = Vec::new();
                match result {
                    Ok(payload) => {
                        response.extend(0u32.to_be_bytes());
                        response.extend(payload);
                    }
                    Err(status) => response.extend((status as u32).to_be_bytes()),
                }
                if write_frame(&mut stream, &response).is_err() {
                    break;
                }
            }
        });
    }
    Ok(())
}

fn enter_shared_request(
    request: &[u8],
    cache: &PrivateAttrCache,
    notifier: &Notifier,
) -> Result<(), i32> {
    let mut input = Reader::new(request);
    let op = input.u8().map_err(errno)?;
    if !matches!(
        op,
        GETATTR
            | MKDIR
            | CREATE
            | OPEN
            | RENAME
            | UNLINK
            | RMDIR
            | READDIR
            | OPENDIR
            | SETLEN
            | SETATTR
    ) {
        return Ok(());
    }
    let path = input.string().map_err(errno)?;
    cache.enter_shared_path(path, notifier)?;
    if op == RENAME {
        let to = input.string().map_err(errno)?;
        cache.enter_shared_path(to, notifier)?;
    }
    Ok(())
}

fn authenticate(bytes: &[u8], token: &str) -> Result<(), i32> {
    let mut input = Reader::new(bytes);
    if input.u8().map_err(errno)? != AUTH {
        return Err(libc::EACCES);
    }
    if input.u32().map_err(errno)? != WIRE_VERSION {
        return Err(libc::EPROTONOSUPPORT);
    }
    let actual = input.bytes().map_err(errno)?;
    input.done().map_err(errno)?;
    if actual == token.as_bytes() {
        Ok(())
    } else {
        Err(libc::EACCES)
    }
}

fn write_status(stream: &mut TcpStream, result: Result<(), i32>) -> io::Result<()> {
    let status = match result {
        Ok(()) => 0,
        Err(errno) => errno,
    };
    write_frame(stream, &(status as u32).to_be_bytes())
}

struct Session {
    root: PathBuf,
    handles: HashMap<u64, File>,
    next_handle: u64,
}
impl Session {
    #[cfg(test)]
    fn parts<'a>(&self, name: &'a str) -> Result<Vec<&'a str>, i32> {
        if !name.starts_with('/') {
            return Err(libc::EINVAL);
        }
        if name == "/" {
            return Ok(Vec::new());
        }
        let mut parts = Vec::new();
        for part in name.split('/').skip(1) {
            if part.is_empty() || part == "." || part == ".." || part.as_bytes().contains(&0) {
                return Err(libc::EINVAL);
            }
            parts.push(part);
        }
        Ok(parts)
    }
    #[cfg(test)]
    fn path(&self, name: &str) -> Result<PathBuf, i32> {
        let mut path = self.root.clone();
        for part in self.parts(name)? {
            path.push(part);
        }
        Ok(path)
    }
    fn attr_at(&self, path: &str) -> Result<Attr, i32> {
        let file = checked_open(&self.root, path, libc::O_PATH, 0)?;
        Attr::from_metadata(&file.metadata().map_err(errno)?)
    }
    fn file_type_at(&self, path: &str) -> Result<fs::FileType, i32> {
        let file = checked_open(&self.root, path, libc::O_PATH, 0)?;
        let meta = file.metadata().map_err(errno)?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            Err(libc::ELOOP)
        } else if kind.is_file() || kind.is_dir() {
            Ok(kind)
        } else {
            Err(libc::EOPNOTSUPP)
        }
    }
    fn checked_range(offset: u64, size: usize) -> Result<(), i32> {
        if offset > i64::MAX as u64 || size > MAX_IO_BYTES {
            return Err(libc::EINVAL);
        }
        offset
            .checked_add(size as u64)
            .filter(|end| *end <= i64::MAX as u64)
            .map(|_| ())
            .ok_or(libc::EINVAL)
    }
    fn apply_setattr(&self, path: &str, spec: &SetAttrSpec) -> Result<(), i32> {
        let (parent, leaf) = open_parent_dir(&self.root, path)?;
        let name = CString::new(leaf).map_err(|_| libc::EINVAL)?;
        if let Some(mode) = spec.mode {
            let result = unsafe {
                libc::fchmodat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    (mode & 0o7777) as libc::mode_t,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result < 0 {
                return Err(errno(io::Error::last_os_error()));
            }
        }
        if spec.uid.is_some() || spec.gid.is_some() {
            let uid = spec.uid.map_or(u32::MAX as libc::uid_t, |uid| uid as _);
            let gid = spec.gid.map_or(u32::MAX as libc::gid_t, |gid| gid as _);
            let result = unsafe {
                libc::fchownat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    uid,
                    gid,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result < 0 {
                return Err(errno(io::Error::last_os_error()));
            }
        }
        if spec.atime.is_some() || spec.mtime.is_some() {
            let times = [
                TimeSpec::timespec(spec.atime)?,
                TimeSpec::timespec(spec.mtime)?,
            ];
            let result = unsafe {
                libc::utimensat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    times.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result < 0 {
                return Err(errno(io::Error::last_os_error()));
            }
        }
        Ok(())
    }
    fn open_file(&mut self, path: &str, flags: i32, mode: u32) -> Result<u64, i32> {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.checked_add(1).ok_or(libc::EOVERFLOW)?;
        self.handles
            .insert(handle, checked_open(&self.root, path, flags, mode)?);
        Ok(handle)
    }
    fn file(&self, handle: u64) -> Result<&File, i32> {
        self.handles.get(&handle).ok_or(libc::EBADF)
    }
    fn handle(&mut self, bytes: &[u8]) -> Result<Vec<u8>, i32> {
        let mut input = Reader::new(bytes);
        let op = input.u8().map_err(errno)?;
        let mut out = Writer::new(0);
        match op {
            GETATTR => {
                let path = input.string().map_err(errno)?;
                input.done().map_err(errno)?;
                self.attr_at(path)?.write(&mut out);
            }
            MKDIR => {
                let path = input.string().map_err(errno)?;
                let mode = input.u32().map_err(errno)?;
                input.done().map_err(errno)?;
                checked_mkdir(&self.root, path, mode)?;
                self.attr_at(path)?.write(&mut out);
            }
            CREATE => {
                let path = input.string().map_err(errno)?;
                let flags = input.u32().map_err(errno)? as i32;
                let mode = input.u32().map_err(errno)?;
                input.done().map_err(errno)?;
                let handle = self.open_file(path, flags | libc::O_CREAT, mode)?;
                out.u64(handle);
                Attr::from_metadata(&self.file(handle)?.metadata().map_err(errno)?)?
                    .write(&mut out);
            }
            OPEN | OPENDIR => {
                let path = input.string().map_err(errno)?;
                let mut flags = input.u32().map_err(errno)? as i32;
                input.done().map_err(errno)?;
                let file_type = self.file_type_at(path)?;
                if op == OPENDIR {
                    if !file_type.is_dir() {
                        return Err(libc::ENOTDIR);
                    }
                    flags |= libc::O_DIRECTORY;
                } else if !file_type.is_file() {
                    return Err(libc::EOPNOTSUPP);
                }
                let handle = self.open_file(path, flags, 0)?;
                out.u64(handle);
                let attr = Attr::from_metadata(&self.file(handle)?.metadata().map_err(errno)?)?;
                attr.write(&mut out);
                if op == OPEN
                    && flags & libc::O_ACCMODE == libc::O_RDONLY
                    && flags & (libc::O_PATH | libc::O_DIRECT) == 0
                    && attr.size <= OPEN_PREFETCH_LIMIT as u64
                {
                    let mut bytes = vec![0; attr.size as usize];
                    let read = self.file(handle)?.read_at(&mut bytes, 0).map_err(errno)?;
                    if read == bytes.len() {
                        out.u8(1);
                        out.bytes(&bytes);
                    } else {
                        out.u8(0);
                    }
                } else {
                    out.u8(0);
                }
            }
            READ => {
                let handle = input.u64().map_err(errno)?;
                let offset = input.u64().map_err(errno)?;
                let size = input.u32().map_err(errno)? as usize;
                input.done().map_err(errno)?;
                Self::checked_range(offset, size)?;
                let mut buffer = vec![0; size];
                let count = self
                    .file(handle)?
                    .read_at(&mut buffer, offset)
                    .map_err(errno)?;
                out.bytes(&buffer[..count]);
            }
            WRITE => {
                let handle = input.u64().map_err(errno)?;
                let offset = input.u64().map_err(errno)?;
                let data = input.bytes().map_err(errno)?;
                input.done().map_err(errno)?;
                Self::checked_range(offset, data.len())?;
                out.u32(self.file(handle)?.write_at(data, offset).map_err(errno)? as u32);
                Attr::from_metadata(&self.file(handle)?.metadata().map_err(errno)?)?
                    .write(&mut out);
            }
            FSYNC => {
                let handle = input.u64().map_err(errno)?;
                let datasync = input.u8().map_err(errno)? != 0;
                input.done().map_err(errno)?;
                if datasync {
                    self.file(handle)?.sync_data().map_err(errno)?;
                } else {
                    self.file(handle)?.sync_all().map_err(errno)?;
                }
            }
            CLOSE | CLOSE_NO_REPLY => {
                let handle = input.u64().map_err(errno)?;
                input.done().map_err(errno)?;
                let file = self.handles.remove(&handle).ok_or(libc::EBADF)?;
                let descriptor = file.into_raw_fd();
                if unsafe { libc::close(descriptor) } < 0 {
                    return Err(errno(io::Error::last_os_error()));
                }
            }
            RENAME => {
                let from = input.string().map_err(errno)?;
                let to = input.string().map_err(errno)?;
                input.done().map_err(errno)?;
                self.file_type_at(from)?;
                if let Err(error) = self.file_type_at(to)
                    && error != libc::ENOENT
                {
                    return Err(error);
                }
                checked_rename(&self.root, from, to)?;
                self.attr_at(to)?.write(&mut out);
            }
            UNLINK | RMDIR => {
                let path = input.string().map_err(errno)?;
                input.done().map_err(errno)?;
                let file_type = self.file_type_at(path)?;
                if op == UNLINK {
                    if !file_type.is_file() {
                        return Err(libc::EOPNOTSUPP);
                    }
                    checked_unlink(&self.root, path, false)?;
                } else {
                    if !file_type.is_dir() {
                        return Err(libc::ENOTDIR);
                    }
                    checked_unlink(&self.root, path, true)?;
                }
            }
            READDIR => {
                let path = input.string().map_err(errno)?;
                input.done().map_err(errno)?;
                let rows = checked_read_dir(&self.root, path)?;
                out.u32(rows.len() as u32);
                for (name, kind) in rows {
                    out.string(&name);
                    out.u8(if kind == FileType::Directory { 1 } else { 2 });
                }
            }
            SETLEN => {
                let path = input.string().map_err(errno)?;
                let size = input.u64().map_err(errno)?;
                input.done().map_err(errno)?;
                let file_type = self.file_type_at(path)?;
                if !file_type.is_file() {
                    return Err(libc::EOPNOTSUPP);
                }
                let file = checked_open(&self.root, path, libc::O_WRONLY, 0)?;
                file.set_len(size).map_err(errno)?;
                Attr::from_metadata(&file.metadata().map_err(errno)?)?.write(&mut out);
            }
            SETATTR => {
                let path = input.string().map_err(errno)?;
                let spec = SetAttrSpec::read(&mut input).map_err(errno)?;
                input.done().map_err(errno)?;
                self.file_type_at(path)?;
                self.apply_setattr(path, &spec)?;
                self.attr_at(path)?.write(&mut out);
            }
            _ => return Err(libc::ENOSYS),
        }
        Ok(out.finish().split_off(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        os::fd::FromRawFd,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
        thread,
    };

    static NEXT_TMP: AtomicU64 = AtomicU64::new(1);

    fn temp_home(name: &str) -> PathBuf {
        let id = NEXT_TMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dms-home-p2p-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn serve_test(root: PathBuf, token: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        thread::spawn(move || serve_listener(listener, root, token.to_owned()).unwrap());
        address
    }

    #[test]
    fn rejects_paths_outside_home() {
        let state = Session {
            root: PathBuf::from("/tmp"),
            handles: HashMap::new(),
            next_handle: 1,
        };
        for path in ["../x", "/../x", "/a/../../x", "/a//b", "/a/./b"] {
            assert_eq!(state.path(path).unwrap_err(), libc::EINVAL);
        }
        assert_eq!(state.path("/a/b").unwrap(), PathBuf::from("/tmp/a/b"));
    }
    #[test]
    fn frame_reader_rejects_short_or_extra_input() {
        assert!(Reader::new(&[0, 1]).u32().is_err());
        let mut input = Reader::new(&[7, 8]);
        assert_eq!(input.u8().unwrap(), 7);
        assert!(input.done().is_err());
    }

    #[test]
    fn authenticated_connect_rejects_wrong_token() {
        let root = temp_home("auth");
        let address = serve_test(root.clone(), "secret");

        assert!(Client::connect_authenticated(&address, "bad").is_err());
        let mut client = Client::connect_authenticated(&address, "secret").unwrap();
        let attr = client.mkdir("/ok", 0o755).unwrap();
        assert_eq!(attr.kind, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn authentication_rejects_old_wire_format() {
        let mut old = Writer::new(AUTH);
        old.bytes(b"secret");
        assert!(authenticate(&old.finish(), "secret").is_err());
        let mut wrong_version = Writer::new(AUTH);
        wrong_version.u32(WIRE_VERSION - 1);
        wrong_version.bytes(b"secret");
        assert_eq!(
            authenticate(&wrong_version.finish(), "secret"),
            Err(libc::EPROTONOSUPPORT)
        );
    }

    #[test]
    fn rejects_symlink_escape_and_unsupported_entries() {
        let root = temp_home("symlink");
        let outside = temp_home("outside");
        fs::write(outside.join("secret"), b"hidden").unwrap();
        symlink(&outside, root.join("link")).unwrap();
        symlink(outside.join("secret"), root.join("file-link")).unwrap();

        let mut state = Session {
            root: root.clone(),
            handles: HashMap::new(),
            next_handle: 1,
        };

        let mut open_link = Writer::new(OPEN);
        open_link.string("/link/secret");
        open_link.u32(libc::O_RDONLY as u32);
        assert!(matches!(
            state.handle(&open_link.finish()),
            Err(libc::ELOOP | libc::ENOTDIR)
        ));

        let mut getattr_link = Writer::new(GETATTR);
        getattr_link.string("/file-link");
        assert_eq!(
            state.handle(&getattr_link.finish()).unwrap_err(),
            libc::ELOOP
        );

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn held_parent_fd_cannot_follow_swapped_symlink() {
        let root = temp_home("parent-swap");
        let outside = temp_home("parent-swap-outside");
        fs::create_dir(root.join("safe")).unwrap();
        fs::write(root.join("safe/note"), b"inside").unwrap();
        fs::write(outside.join("note"), b"outside").unwrap();
        let (parent, leaf) = open_parent_dir(&root, "/safe/note").unwrap();
        fs::rename(root.join("safe"), root.join("old-safe")).unwrap();
        symlink(&outside, root.join("safe")).unwrap();
        let name = CString::new(leaf).unwrap();
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW,
            )
        };
        assert!(fd >= 0);
        let mut file = unsafe { File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"inside");
        assert!(matches!(
            checked_open(&root, "/safe/note", libc::O_RDONLY, 0),
            Err(libc::ELOOP | libc::ENOTDIR)
        ));
        assert_eq!(fs::read(outside.join("note")).unwrap(), b"outside");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn rejects_overlarge_io_and_bad_close_handle() {
        let root = temp_home("bounds");
        let mut state = Session {
            root: root.clone(),
            handles: HashMap::new(),
            next_handle: 1,
        };

        let mut read = Writer::new(READ);
        read.u64(123);
        read.u64(0);
        read.u32((MAX_IO_BYTES + 1) as u32);
        assert_eq!(state.handle(&read.finish()).unwrap_err(), libc::EINVAL);

        let mut close = Writer::new(CLOSE);
        close.u64(123);
        assert_eq!(state.handle(&close.finish()).unwrap_err(), libc::EBADF);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reconnect_preserves_close_to_open_bytes() {
        let root = temp_home("reconnect");
        let address = serve_test(root.clone(), "secret");

        let mut first = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _) = first.create("/note.txt", libc::O_RDWR, 0o644).unwrap();
        assert_eq!(first.write(handle, 0, b"new-bytes").unwrap().0, 9);
        first.fsync(handle, false).unwrap();
        first.close(handle).unwrap();
        drop(first);

        let mut second = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _, prefetched) = second.open("/note.txt", libc::O_RDONLY, false).unwrap();
        assert_eq!(prefetched.as_deref(), Some(b"new-bytes".as_slice()));
        assert_eq!(second.read(handle, 0, 64).unwrap(), b"new-bytes");
        second.close(handle).unwrap();

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn small_read_prefetch_is_scoped_to_one_open() {
        let root = temp_home("prefetch-reopen");
        let address = serve_test(root.clone(), "secret");
        let mut writer = Client::connect_authenticated(&address, "secret").unwrap();
        let mut reader = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _) = writer.create("/note", libc::O_RDWR, 0o644).unwrap();
        writer.write(handle, 0, b"first").unwrap();
        writer.fsync(handle, true).unwrap();
        writer.close(handle).unwrap();

        let (handle, _, bytes) = reader.open("/note", libc::O_RDONLY, false).unwrap();
        assert_eq!(bytes.as_deref(), Some(b"first".as_slice()));
        reader.close(handle).unwrap();

        let (handle, _, _) = writer.open("/note", libc::O_WRONLY, false).unwrap();
        writer.write(handle, 0, b"newer").unwrap();
        writer.fsync(handle, true).unwrap();
        writer.close(handle).unwrap();
        let (handle, _, bytes) = reader.open("/note", libc::O_RDONLY, false).unwrap();
        assert_eq!(bytes.as_deref(), Some(b"newer".as_slice()));
        reader.close(handle).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn one_way_read_close_keeps_next_reply_aligned() {
        let root = temp_home("one-way-close");
        fs::write(root.join("note"), b"content").unwrap();
        let address = serve_test(root.clone(), "secret");
        let mut client = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _, _) = client.open("/note", libc::O_RDONLY, false).unwrap();
        client.close_no_reply(handle).unwrap();
        assert_eq!(client.getattr("/note").unwrap().size, 7);
        assert_eq!(client.close(handle), Err(libc::EBADF));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn path_only_open_does_not_try_to_prefetch() {
        let root = temp_home("path-only-open");
        fs::write(root.join("note"), b"content").unwrap();
        let address = serve_test(root.clone(), "secret");
        let mut client = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _, prefetched) = client.open("/note", libc::O_PATH, false).unwrap();
        assert!(prefetched.is_none());
        client.close(handle).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn setattr_changes_mode_and_mtime_without_truncating() {
        let root = temp_home("setattr-client");
        let address = serve_test(root.clone(), "secret");

        let mut client = Client::connect_authenticated(&address, "secret").unwrap();
        let (handle, _) = client.create("/touch.txt", libc::O_RDWR, 0o644).unwrap();
        client.write(handle, 0, b"content").unwrap();
        client.close(handle).unwrap();

        let attr = client
            .setattr(
                "/touch.txt",
                &SetAttrSpec {
                    mode: Some(0o600),
                    uid: None,
                    gid: None,
                    atime: Some(TimeSpec::Now),
                    mtime: Some(TimeSpec::At(1_700_000_001, 123_000_000)),
                },
            )
            .unwrap();

        assert_eq!(attr.perm & 0o777, 0o600);
        assert_eq!(attr.size, 7);
        assert_eq!(attr.mtime, 1_700_000_001);
        assert_eq!(attr.mtime_ns, 123_000_000);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn setattr_rejects_symlink_and_invalid_time() {
        let root = temp_home("setattr-bounds");
        let outside = temp_home("setattr-outside");
        fs::write(outside.join("secret"), b"hidden").unwrap();
        symlink(outside.join("secret"), root.join("link")).unwrap();
        fs::write(root.join("regular"), b"ok").unwrap();

        let mut state = Session {
            root: root.clone(),
            handles: HashMap::new(),
            next_handle: 1,
        };

        let mut symlink_req = Writer::new(SETATTR);
        symlink_req.string("/link");
        SetAttrSpec {
            mode: Some(0o600),
            uid: None,
            gid: None,
            atime: None,
            mtime: None,
        }
        .write(&mut symlink_req);
        assert_eq!(
            state.handle(&symlink_req.finish()).unwrap_err(),
            libc::ELOOP
        );

        let mut bad_time = Writer::new(SETATTR);
        bad_time.string("/regular");
        SetAttrSpec {
            mode: None,
            uid: None,
            gid: None,
            atime: None,
            mtime: Some(TimeSpec::At(1, 1_000_000_000)),
        }
        .write(&mut bad_time);
        assert_eq!(state.handle(&bad_time.finish()).unwrap_err(), libc::EINVAL);

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
