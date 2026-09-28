//! Safe local filesystem backend for node-owned data directories.
//!
//! LocalFs is the first concrete `FileStore` implementation. It is intentionally
//! small: it provides root-confined local POSIX primitives without WAL, ownership
//! state, image publication, or strong file-identity protection. Those rules live
//! in OwnerFs/DFS above this layer.

use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, FileExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};

use super::{DirectoryHandle, FileHandle, FileStore, OpenSpec, RenameMode, StoragePath};

const UNSUPPORTED_RENAME_MODE: &str = "rename mode requires unsupported renameat2 semantics";

/// Root-confined local disk store.
///
/// The root directory is opened once and all child operations are resolved from
/// that descriptor via `/proc/self/fd/<fd>/<child>`. Each traversed directory is
/// opened with `O_NOFOLLOW | O_DIRECTORY`; file opens add `O_NOFOLLOW` for the
/// leaf. This rejects symlink traversal without using unsafe Rust or adding an
/// `openat2` wrapper dependency.
#[derive(Debug)]
pub struct LocalFs {
    root: File,
    root_path: PathBuf,
}

impl LocalFs {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root_path = root.as_ref().canonicalize()?;
        let root = open_dir_path(&root_path)?;
        Ok(Self { root, root_path })
    }

    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    fn open_parent(&self, path: &StoragePath) -> io::Result<(File, OsString)> {
        let mut components = split_components(path)?;
        let leaf = components.pop().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "operation requires a non-root storage path",
            )
        })?;
        let parent = self.open_dir_components(&components)?;
        Ok((parent, leaf))
    }

    fn open_dir_components(&self, components: &[OsString]) -> io::Result<File> {
        let mut dir = self.root.try_clone()?;
        for component in components {
            dir = open_child_dir(&dir, component)?;
        }
        Ok(dir)
    }

    fn open_path_dir(&self, path: &StoragePath) -> io::Result<File> {
        if path.is_root() {
            return self.root.try_clone();
        }
        self.open_dir_components(&split_components(path)?)
    }
}

impl FileStore for LocalFs {
    type File = LocalFile;
    type Directory = LocalDirectory;

    fn open_file(&self, path: &StoragePath, spec: OpenSpec) -> io::Result<Self::File> {
        let (parent, leaf) = self.open_parent(path)?;
        let file = open_child_file(&parent, &leaf, spec)?;
        Ok(LocalFile { file })
    }

    fn open_dir(&self, path: &StoragePath) -> io::Result<Self::Directory> {
        Ok(LocalDirectory {
            dir: self.open_path_dir(path)?,
        })
    }

    fn metadata(&self, path: &StoragePath) -> io::Result<fs::Metadata> {
        if path.is_root() {
            return self.root.metadata();
        }
        let (parent, leaf) = self.open_parent(path)?;
        // Attribute lookup does not follow the final symlink. Intermediate
        // symlinks have already been rejected by `open_parent`.
        fs::symlink_metadata(proc_child(&parent, &leaf))
    }

    fn read_dir(&self, path: &StoragePath) -> io::Result<Vec<OsString>> {
        self.open_dir(path)?.read_dir()
    }

    fn mkdir(&self, path: &StoragePath, mode: u32) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(path)?;
        fs::DirBuilder::new()
            .mode(mode)
            .create(proc_child(&parent, &leaf))
    }

    fn remove_file(&self, path: &StoragePath) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(path)?;
        fs::remove_file(proc_child(&parent, &leaf))
    }

    fn remove_dir(&self, path: &StoragePath) -> io::Result<()> {
        let (parent, leaf) = self.open_parent(path)?;
        fs::remove_dir(proc_child(&parent, &leaf))
    }

    fn rename(&self, from: &StoragePath, to: &StoragePath, mode: RenameMode) -> io::Result<()> {
        if mode == RenameMode::Exchange {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                UNSUPPORTED_RENAME_MODE,
            ));
        }
        let (from_parent, from_leaf) = self.open_parent(from)?;
        let (to_parent, to_leaf) = self.open_parent(to)?;
        match mode {
            RenameMode::Replace => fs::rename(
                proc_child(&from_parent, &from_leaf),
                proc_child(&to_parent, &to_leaf),
            ),
            RenameMode::NoReplace => {
                renameat2_no_replace(&from_parent, &from_leaf, &to_parent, &to_leaf)
            }
            RenameMode::Exchange => unreachable!("Exchange returned above"),
        }
    }

    fn sync_root(&self) -> io::Result<()> {
        self.root.sync_all()
    }
}

/// Plain local file handle.
///
/// There is no userspace buffer in this implementation, so `flush` is a no-op.
/// Durable boundaries remain explicit: `sync_data` for file content and minimal
/// metadata, `sync_all` for full file metadata.
#[derive(Debug)]
pub struct LocalFile {
    file: File,
}

impl FileHandle for LocalFile {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read_at(buffer, offset)
    }

    fn write_at(&self, offset: u64, buffer: &[u8]) -> io::Result<usize> {
        self.file.write_at(buffer, offset)
    }

    fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }

    fn set_len(&self, size: u64) -> io::Result<()> {
        self.file.set_len(size)
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()
    }
}

#[derive(Debug)]
pub struct LocalDirectory {
    dir: File,
}

impl DirectoryHandle for LocalDirectory {
    fn metadata(&self) -> io::Result<fs::Metadata> {
        self.dir.metadata()
    }

    fn read_dir(&self) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(proc_fd_path(&self.dir))? {
            let entry = entry?;
            let name = entry.file_name();
            if name != OsStr::new(".") && name != OsStr::new("..") {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn sync_all(&self) -> io::Result<()> {
        self.dir.sync_all()
    }
}

fn split_components(path: &StoragePath) -> io::Result<Vec<OsString>> {
    let bytes = path.bytes();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut components = Vec::new();
    for component in bytes.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." || component.contains(&0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "storage path contains an invalid component",
            ));
        }
        components.push(OsString::from_vec(component.to_vec()));
    }
    Ok(components)
}

fn open_dir_path(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn open_child_dir(parent: &File, leaf: &OsStr) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(proc_child(parent, leaf))
}

fn open_child_file(parent: &File, leaf: &OsStr, spec: OpenSpec) -> io::Result<File> {
    if spec.flags & libc::O_DIRECTORY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "open_file does not accept O_DIRECTORY",
        ));
    }

    let mut options = OpenOptions::new();
    match spec.flags & libc::O_ACCMODE {
        libc::O_RDONLY => {
            options.read(true);
        }
        libc::O_WRONLY => {
            options.write(true);
        }
        libc::O_RDWR => {
            options.read(true).write(true);
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported access mode",
            ));
        }
    }
    options
        .create(spec.flags & libc::O_CREAT != 0)
        .create_new(spec.flags & libc::O_CREAT != 0 && spec.flags & libc::O_EXCL != 0)
        .truncate(spec.flags & libc::O_TRUNC != 0)
        .append(spec.flags & libc::O_APPEND != 0)
        .mode(spec.mode)
        .custom_flags(passthrough_flags(spec.flags) | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(proc_child(parent, leaf))
}

fn passthrough_flags(flags: i32) -> i32 {
    flags
        & !(libc::O_ACCMODE
            | libc::O_CREAT
            | libc::O_EXCL
            | libc::O_TRUNC
            | libc::O_APPEND
            | libc::O_DIRECTORY)
}

fn proc_child(parent: &File, leaf: &OsStr) -> PathBuf {
    proc_fd_path(parent).join(leaf)
}

#[allow(unsafe_code)]
fn renameat2_no_replace(
    from_parent: &File,
    from_leaf: &OsStr,
    to_parent: &File,
    to_leaf: &OsStr,
) -> io::Result<()> {
    let from = CString::new(from_leaf.as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "source name contains NUL byte")
    })?;
    let to = CString::new(to_leaf.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination name contains NUL byte",
        )
    })?;
    // SAFETY: parent descriptors are live directory FDs opened by LocalFs.
    // `from` and `to` are NUL-terminated single path components validated by
    // StoragePath/open_parent. The syscall does not retain these pointers after
    // returning, and RENAME_NOREPLACE gives the required atomic no-clobber
    // semantics that cannot be implemented with a prior existence check.
    let result = unsafe {
        libc::renameat2(
            from_parent.as_raw_fd(),
            from.as_ptr(),
            to_parent.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn proc_fd_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}
