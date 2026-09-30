//! Descriptor-confined Linux export backend.
//!
//! The namespace and parent directory must be exclusive to the managed
//! lifecycle writer. Native handles are not revoked by this module. The
//! caller must quiesce/drain managed users before normal unmount and reclaim.
use super::{DirectoryIdentity, MountBackend, MountIdentity, NamespaceIdentity, WorkspaceMount};
use std::{
    collections::HashMap,
    ffi::{CString, OsStr},
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    sync::{Mutex, MutexGuard},
};

const OPEN_TREE_CLONE: u32 = 1;
const MOVE_MOUNT_F_EMPTY_PATH: u32 = 0x4;
const MOVE_MOUNT_T_EMPTY_PATH: u32 = 0x40;
const MOUNT_ATTR_RDONLY: u64 = 0x1;
const MOUNT_ATTR_NOSUID: u64 = 0x2;
const MOUNT_ATTR_NODEV: u64 = 0x4;
const MOUNT_ATTR_NOEXEC: u64 = 0x8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MountPolicy {
    pub read_only: bool,
    pub no_exec: bool,
}

struct Prepared {
    spec: WorkspaceMount,
    source: File,
    parent: File,
    _covered_target: File,
    name: CString,
    covered_unique_mount_id: u64,
    policy: MountPolicy,
    attached: Option<MountIdentity>,
}

pub struct LinuxMountBackend {
    namespace: NamespaceIdentity,
    capacity: usize,
    prepared: Mutex<HashMap<String, Prepared>>,
}

impl LinuxMountBackend {
    pub fn current_namespace() -> io::Result<NamespaceIdentity> {
        let metadata = File::open("/proc/thread-self/ns/mnt")?.metadata()?;
        Ok(NamespaceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub fn new(namespace: NamespaceIdentity, capacity: usize) -> io::Result<Self> {
        if namespace != Self::current_namespace()? {
            return Err(errno(libc::ESTALE));
        }
        if capacity == 0 || capacity > 4096 {
            return Err(errno(libc::EINVAL));
        }
        Ok(Self {
            namespace,
            capacity,
            prepared: Mutex::new(HashMap::new()),
        })
    }

    /// Admission supplies trusted root-grant-derived descriptors. The single
    /// target component is interpreted relative to the pinned managed parent.
    /// Preparation never adopts an existing mount as owned.
    pub fn prepare(
        &self,
        spec: WorkspaceMount,
        source: File,
        parent: File,
        name: &OsStr,
        policy: MountPolicy,
    ) -> io::Result<()> {
        self.check_namespace()?;
        let bytes = name.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || bytes == b"."
            || bytes == b".."
            || bytes.contains(&b'/')
        {
            return Err(errno(libc::EINVAL));
        }
        let name = CString::new(bytes).map_err(|_| errno(libc::EINVAL))?;
        if spec.identity.namespace != self.namespace
            || directory_identity(&source)? != spec.source
            || !parent.metadata()?.is_dir()
        {
            return Err(errno(libc::ESTALE));
        }
        let target = open_child(&parent, &name)?;
        if directory_identity(&target)? != spec.target {
            return Err(errno(libc::ESTALE));
        }
        let covered_unique_mount_id = unique_mount_id(&target)?;
        let mut prepared = lock(&self.prepared)?;
        if let Some(old) = prepared.get(&spec.identity.root_id) {
            // Do not replace active descriptors or reset recorded ownership.
            if old.spec == spec
                && old.policy == policy
                && old.name == name
                && directory_identity(&old.parent)? == directory_identity(&parent)?
            {
                return Ok(());
            }
            return Err(errno(libc::EEXIST));
        }
        if prepared.len() >= self.capacity {
            return Err(errno(libc::ENOSPC));
        }
        prepared.insert(
            spec.identity.root_id.clone(),
            Prepared {
                spec,
                source,
                parent,
                _covered_target: target,
                name,
                covered_unique_mount_id,
                policy,
                attached: None,
            },
        );
        Ok(())
    }

    fn check_namespace(&self) -> io::Result<()> {
        if Self::current_namespace()? != self.namespace {
            return Err(errno(libc::ESTALE));
        }
        Ok(())
    }

    fn inspect_prepared(&self, entry: &Prepared) -> io::Result<Option<MountIdentity>> {
        if directory_identity(&entry.source)? != entry.spec.source {
            return Err(errno(libc::ESTALE));
        }
        let top = open_child(&entry.parent, &entry.name)?;
        let actual_id = mount_id(&top)?;
        let actual_unique_id = unique_mount_id(&top)?;
        let actual_directory = directory_identity(&top)?;
        if actual_unique_id == entry.covered_unique_mount_id {
            if actual_directory != entry.spec.target {
                return Err(errno(libc::ESTALE));
            }
            return Ok(None);
        }
        // Foreign/stacked observations are returned for diagnosis. Ownership
        // is separately checked against this backend's successful attach.
        Ok(Some(MountIdentity {
            mount_id: actual_id,
            unique_mount_id: actual_unique_id,
            namespace: self.namespace,
            source: actual_directory,
            covered_target: entry.spec.target,
        }))
    }
}

impl MountBackend for LinuxMountBackend {
    fn inspect(&self, spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        self.check_namespace()?;
        let records = lock(&self.prepared)?;
        let entry = get(&records, spec)?;
        self.inspect_prepared(entry)
    }

    fn attached_claim(&self, spec: &WorkspaceMount) -> io::Result<Option<MountIdentity>> {
        self.check_namespace()?;
        let records = lock(&self.prepared)?;
        Ok(get(&records, spec)?.attached.clone())
    }

    fn bind(&self, spec: &WorkspaceMount) -> io::Result<MountIdentity> {
        self.check_namespace()?;
        let mut records = lock(&self.prepared)?;
        let entry = get_mut(&mut records, spec)?;
        if entry.attached.is_some() || self.inspect_prepared(entry)?.is_some() {
            return Err(errno(libc::EEXIST));
        }
        let target = open_child(&entry.parent, &entry.name)?;
        if unique_mount_id(&target)? != entry.covered_unique_mount_id
            || directory_identity(&target)? != spec.target
        {
            return Err(errno(libc::ESTALE));
        }
        let tree = clone_tree(&entry.source)?;
        apply_policy(&tree, entry.policy)?;
        // Identity is known before attachment, retaining the claim if later
        // observation fails. Caller must reconcile instead of retry stacking.
        let cloned = MountIdentity {
            mount_id: mount_id(&tree)?,
            unique_mount_id: unique_mount_id(&tree)?,
            namespace: self.namespace,
            source: directory_identity(&tree)?,
            covered_target: spec.target,
        };
        if !cloned.matches(spec) {
            return Err(errno(libc::ESTALE));
        }
        attach_tree(&tree, &target)?;
        entry.attached = Some(cloned.clone());
        drop(tree);
        drop(target);
        if self.inspect_prepared(entry)?.as_ref() != Some(&cloned) {
            return Err(errno(libc::ESTALE));
        }
        Ok(cloned)
    }

    fn unmount(&self, spec: &WorkspaceMount, expected: &MountIdentity) -> io::Result<()> {
        self.check_namespace()?;
        let mut records = lock(&self.prepared)?;
        let entry = get_mut(&mut records, spec)?;
        if entry.attached.as_ref() != Some(expected)
            || !expected.matches(spec)
            || self.inspect_prepared(entry)?.as_ref() != Some(expected)
        {
            return Err(errno(libc::ESTALE));
        }
        // No fd referring to the attached mount is kept open here. The pinned
        // parent is below the covered FUSE root, not inside the native mount.
        // Exclusive namespace/lifecycle ownership prevents competing writers
        // between inspection and this path-based normal umount syscall.
        let mut bytes = format!("/proc/thread-self/fd/{}/", entry.parent.as_raw_fd()).into_bytes();
        bytes.extend_from_slice(entry.name.as_bytes());
        let path = CString::new(bytes).map_err(|_| errno(libc::EINVAL))?;
        normal_unmount(&path)?;
        if self.inspect_prepared(entry)?.is_some() {
            return Err(errno(libc::ESTALE));
        }
        entry.attached = None;
        Ok(())
    }
}

fn directory_identity(file: &File) -> io::Result<DirectoryIdentity> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(errno(libc::ENOTDIR));
    }
    Ok(DirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
fn get<'a>(
    records: &'a HashMap<String, Prepared>,
    spec: &WorkspaceMount,
) -> io::Result<&'a Prepared> {
    let entry = records
        .get(&spec.identity.root_id)
        .ok_or_else(|| errno(libc::ENOENT))?;
    if entry.spec != *spec {
        return Err(errno(libc::ESTALE));
    }
    Ok(entry)
}
fn get_mut<'a>(
    records: &'a mut HashMap<String, Prepared>,
    spec: &WorkspaceMount,
) -> io::Result<&'a mut Prepared> {
    let entry = records
        .get_mut(&spec.identity.root_id)
        .ok_or_else(|| errno(libc::ENOENT))?;
    if entry.spec != *spec {
        return Err(errno(libc::ESTALE));
    }
    Ok(entry)
}
fn errno(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("native descriptors poisoned"))
}
#[allow(unsafe_code)]
fn owned_fd(fd: RawFd) -> io::Result<File> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: all callers transfer a fresh successful syscall result exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[allow(unsafe_code)]
fn open_child(parent: &File, name: &std::ffi::CStr) -> io::Result<File> {
    // SAFETY: parent is live, name is NUL-terminated, openat retains no pointers.
    owned_fd(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })
}
fn mount_id(file: &File) -> io::Result<u64> {
    statx_mount_id(file, libc::STATX_MNT_ID)
}
fn unique_mount_id(file: &File) -> io::Result<u64> {
    // Linux UAPI STATX_MNT_ID_UNIQUE, available from 6.8. Refuse native
    // admission if unsupported rather than substitute a recyclable ID.
    statx_mount_id(file, 0x4000)
}
#[allow(unsafe_code)]
fn statx_mount_id(file: &File, mask: u32) -> io::Result<u64> {
    // SAFETY: statx output is fully zero-initialized writable storage; the fd
    // and empty NUL-terminated string remain live for this synchronous syscall.
    let mut stat: libc::statx = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH,
            mask,
            &mut stat,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if stat.stx_mask & mask == 0 || stat.stx_mnt_id == 0 {
        return Err(errno(libc::ENOTSUP));
    }
    Ok(stat.stx_mnt_id)
}
#[allow(unsafe_code)]
fn clone_tree(source: &File) -> io::Result<File> {
    // SAFETY: fd and empty NUL-terminated pathname remain live; open_tree
    // returns a newly owned mount fd and retains no userspace pointers.
    owned_fd(unsafe {
        libc::syscall(
            libc::SYS_open_tree,
            source.as_raw_fd(),
            c"".as_ptr(),
            OPEN_TREE_CLONE | libc::O_CLOEXEC as u32 | libc::AT_EMPTY_PATH as u32,
        )
    } as RawFd)
}
#[repr(C)]
struct MountAttr {
    attr_set: u64,
    attr_clr: u64,
    propagation: u64,
    userns_fd: u64,
}
#[allow(unsafe_code)]
fn apply_policy(tree: &File, policy: MountPolicy) -> io::Result<()> {
    let attr = MountAttr {
        attr_set: MOUNT_ATTR_NOSUID
            | MOUNT_ATTR_NODEV
            | if policy.read_only {
                MOUNT_ATTR_RDONLY
            } else {
                0
            }
            | if policy.no_exec { MOUNT_ATTR_NOEXEC } else { 0 },
        attr_clr: 0,
        propagation: 0,
        userns_fd: 0,
    };
    // SAFETY: mount_attr has the Linux UAPI layout, live readable storage and
    // exact size; this syscall retains neither the pathname nor attr pointer.
    let result = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            tree.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH as u32,
            &attr as *const MountAttr,
            std::mem::size_of::<MountAttr>(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[allow(unsafe_code)]
fn attach_tree(tree: &File, target: &File) -> io::Result<()> {
    // SAFETY: both fds and empty NUL-terminated strings remain live; move_mount
    // is synchronous and retains no userspace pointers.
    let result = unsafe {
        libc::syscall(
            libc::SYS_move_mount,
            tree.as_raw_fd(),
            c"".as_ptr(),
            target.as_raw_fd(),
            c"".as_ptr(),
            MOVE_MOUNT_F_EMPTY_PATH | MOVE_MOUNT_T_EMPTY_PATH,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[allow(unsafe_code)]
fn normal_unmount(path: &std::ffi::CStr) -> io::Result<()> {
    // SAFETY: pathname remains NUL-terminated and live; zero flags request
    // normal unmount, preserving the kernel's busy-reference check.
    let result = unsafe { libc::umount2(path.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
