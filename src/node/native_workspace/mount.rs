// Descriptor-confined Linux native workspace export mounts.
//
// The caller supplies an already-authorized Home source descriptor and a
// trusted managed parent descriptor. This module records the current mount
// namespace plus exact directory/mount identities, prepares an owned claim,
// activates it with descriptor-confined mount syscalls, and tears it down only
// while the same claim is still visible.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DirectoryIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NamespaceIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MountIdentity {
    pub(super) namespace: NamespaceIdentity,
    pub(super) mount_id: u64,
    pub(super) unique_mount_id: u64,
    pub(super) source: DirectoryIdentity,
    pub(super) covered_target: DirectoryIdentity,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{DirectoryIdentity, MountIdentity, NamespaceIdentity};
    use std::{
        ffi::{CString, OsStr},
        fs::File,
        io,
        os::{
            fd::{AsRawFd, FromRawFd, RawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };

    const OPEN_TREE_CLONE: u32 = 1;
    const MOVE_MOUNT_F_EMPTY_PATH: u32 = 0x4;
    const MOVE_MOUNT_T_EMPTY_PATH: u32 = 0x40;
    const MOUNT_ATTR_NOSUID: u64 = 0x2;
    const MOUNT_ATTR_NODEV: u64 = 0x4;

    #[derive(Debug)]
    pub(in crate::node::native_workspace) struct ManagedExport {
        namespace: NamespaceIdentity,
        source_identity: DirectoryIdentity,
        target_identity: DirectoryIdentity,
        covered_unique_mount_id: u64,
        mount: Option<MountIdentity>,
        source: File,
        parent: File,
        name: CString,
        activated: bool,
    }

    impl ManagedExport {
        pub(in crate::node::native_workspace) fn prepare(
            source: File,
            parent: File,
            name: &OsStr,
        ) -> io::Result<Self> {
            let namespace = current_namespace()?;
            let source_identity = directory_identity(&source)?;
            if !parent.metadata()?.is_dir() {
                return Err(errno(libc::ENOTDIR));
            }
            let name = validate_component(name)?;
            let target = open_child(&parent, &name)?;
            let target_identity = directory_identity(&target)?;
            let covered_unique_mount_id = unique_mount_id(&target)?;
            if covered_unique_mount_id != unique_mount_id(&parent)? {
                return Err(errno(libc::ESTALE));
            }

            Ok(Self {
                namespace,
                source_identity,
                target_identity,
                covered_unique_mount_id,
                mount: None,
                source,
                parent,
                name,
                activated: false,
            })
        }

        pub(in crate::node::native_workspace) fn activate(&mut self) -> io::Result<()> {
            if self.activated {
                return Ok(());
            }

            let tree = clone_tree(&self.source)?;
            apply_policy(&tree)?;
            let mount = MountIdentity {
                namespace: self.namespace,
                mount_id: mount_id(&tree)?,
                unique_mount_id: unique_mount_id(&tree)?,
                source: directory_identity(&tree)?,
                covered_target: self.target_identity,
            };
            if mount.source != self.source_identity {
                return Err(errno(libc::ESTALE));
            }
            self.mount = Some(mount);

            let fresh_target = self.recheck_prepared_target()?;
            attach_tree(&tree, &fresh_target)?;
            self.activated = true;
            drop(tree);
            drop(fresh_target);

            if self.inspect()?.as_ref() != Some(&mount) {
                return Err(errno(libc::ESTALE));
            }
            self.check_policy()?;
            Ok(())
        }

        pub(in crate::node::native_workspace) fn detach(&mut self) -> io::Result<()> {
            if !self.activated {
                if self.inspect()?.is_none() {
                    return Ok(());
                }
                return Err(errno(libc::ESTALE));
            }
            let mount = self.mount_identity()?;
            if directory_identity(&self.source)? != self.source_identity {
                return Err(errno(libc::ESTALE));
            }
            if self.inspect()?.as_ref() != Some(&mount) {
                return Err(errno(libc::ESTALE));
            }
            normal_unmount(&self.target_proc_path()?)?;
            if self.inspect()?.is_some() {
                return Err(errno(libc::ESTALE));
            }
            self.activated = false;
            Ok(())
        }

        pub(in crate::node::native_workspace) fn mount_identity(
            &self,
        ) -> io::Result<MountIdentity> {
            self.mount.ok_or_else(|| errno(libc::ESTALE))
        }

        pub(in crate::node::native_workspace) fn source_identity(&self) -> DirectoryIdentity {
            self.source_identity
        }

        fn recheck_prepared_target(&self) -> io::Result<File> {
            self.check_current_namespace()?;
            if directory_identity(&self.source)? != self.source_identity {
                return Err(errno(libc::ESTALE));
            }
            let target = open_child(&self.parent, &self.name)?;
            if unique_mount_id(&target)? != self.covered_unique_mount_id
                || directory_identity(&target)? != self.target_identity
            {
                return Err(errno(libc::ESTALE));
            }
            Ok(target)
        }

        fn check_current_namespace(&self) -> io::Result<()> {
            if current_namespace()? == self.namespace {
                Ok(())
            } else {
                Err(errno(libc::ESTALE))
            }
        }

        fn inspect(&self) -> io::Result<Option<MountIdentity>> {
            self.check_current_namespace()?;
            let target = open_child(&self.parent, &self.name)?;
            let actual_unique_id = unique_mount_id(&target)?;
            let actual_directory = directory_identity(&target)?;
            if actual_unique_id == self.covered_unique_mount_id {
                if actual_directory != self.target_identity {
                    return Err(errno(libc::ESTALE));
                }
                return Ok(None);
            }
            Ok(Some(MountIdentity {
                namespace: self.namespace,
                mount_id: mount_id(&target)?,
                unique_mount_id: actual_unique_id,
                source: actual_directory,
                covered_target: self.target_identity,
            }))
        }

        fn check_policy(&self) -> io::Result<()> {
            let target = open_child(&self.parent, &self.name)?;
            check_mount_policy(&target)
        }

        fn target_proc_path(&self) -> io::Result<CString> {
            let mut bytes =
                format!("/proc/thread-self/fd/{}/", self.parent.as_raw_fd()).into_bytes();
            bytes.extend_from_slice(self.name.as_bytes());
            CString::new(bytes).map_err(|_| errno(libc::EINVAL))
        }
    }

    pub(in crate::node::native_workspace) fn current_namespace() -> io::Result<NamespaceIdentity> {
        let metadata = File::open("/proc/thread-self/ns/mnt")?.metadata()?;
        namespace_identity_from_metadata(&metadata)
    }

    fn namespace_identity(file: &File) -> io::Result<NamespaceIdentity> {
        namespace_identity_from_metadata(&file.metadata()?)
    }

    fn namespace_identity_from_metadata(
        metadata: &std::fs::Metadata,
    ) -> io::Result<NamespaceIdentity> {
        Ok(NamespaceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
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

    pub(in crate::node::native_workspace) fn inspect_final_clone(
        namespace: &File,
        root: &File,
    ) -> io::Result<(NamespaceIdentity, DirectoryIdentity, u64)> {
        in_mount_namespace(namespace, || {
            let namespace_identity = namespace_identity(namespace)?;
            let target = open_child(root, workspace_name())?;
            check_mount_policy(&target)?;
            Ok((
                namespace_identity,
                directory_identity(&target)?,
                unique_mount_id(&target)?,
            ))
        })
    }

    pub(in crate::node::native_workspace) fn detach_final_clone(
        namespace: &File,
        root: &File,
        expected_source: DirectoryIdentity,
        expected_unique_mount_id: u64,
        covered: DirectoryIdentity,
    ) -> io::Result<()> {
        in_mount_namespace(namespace, || {
            let target = open_child(root, workspace_name())?;
            if directory_identity(&target)? != expected_source
                || unique_mount_id(&target)? != expected_unique_mount_id
            {
                return Err(errno(libc::ESTALE));
            }
            check_mount_policy(&target)?;
            drop(target);
            change_directory(root)?;
            normal_unmount(workspace_name())?;
            let restored = open_child(root, workspace_name())?;
            if directory_identity(&restored)? != covered
                || unique_mount_id(&restored)? == expected_unique_mount_id
            {
                return Err(errno(libc::ESTALE));
            }
            Ok(())
        })
    }

    #[allow(unsafe_code)]
    fn check_mount_policy(target: &File) -> io::Result<()> {
        // Query the mount held by this descriptor, including per-mount flags.
        // A stopped container's procfs cannot resolve this host thread.
        let mut attributes: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor and output storage remain live for the call.
        if unsafe { libc::fstatvfs(target.as_raw_fd(), &mut attributes) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let required = libc::ST_NOSUID | libc::ST_NODEV;
        if attributes.f_flag & required == required {
            Ok(())
        } else {
            Err(errno(libc::EPERM))
        }
    }

    fn workspace_name() -> &'static std::ffi::CStr {
        c"workspace"
    }

    fn in_mount_namespace<T>(
        namespace: &File,
        body: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let original = File::open("/proc/thread-self/ns/mnt")?;
        let original_directory = File::open(".")?;
        // A successful setns with CLONE_NEWNS enters this exact held namespace
        // descriptor. Do not consult the container's procfs after entry.
        setns_mount(namespace)?;
        let result = body();
        let restore = setns_mount(&original).and_then(|_| change_directory(&original_directory));
        match (result, restore) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (_, Err(error)) => Err(error),
        }
    }

    #[allow(unsafe_code)]
    fn change_directory(directory: &File) -> io::Result<()> {
        // SAFETY: the directory descriptor is owned and live. This controller
        // thread has its own CLONE_FS context; the outer scope restores cwd.
        if unsafe { libc::fchdir(directory.as_raw_fd()) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[allow(unsafe_code)]
    fn setns_mount(namespace: &File) -> io::Result<()> {
        let result = unsafe { libc::setns(namespace.as_raw_fd(), libc::CLONE_NEWNS) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn validate_component(name: &OsStr) -> io::Result<CString> {
        let bytes = name.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || bytes == b"."
            || bytes == b".."
            || bytes.contains(&b'/')
        {
            return Err(errno(libc::EINVAL));
        }
        CString::new(bytes).map_err(|_| errno(libc::EINVAL))
    }

    fn errno(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[allow(unsafe_code)]
    fn owned_fd(fd: RawFd) -> io::Result<File> {
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: callers pass a fresh successful syscall/open result exactly once.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    #[allow(unsafe_code)]
    fn open_child(parent: &File, name: &std::ffi::CStr) -> io::Result<File> {
        // SAFETY: parent and name are live for this synchronous syscall.
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
        // STATX_MNT_ID_UNIQUE is Linux 6.8+; fail closed instead of using a
        // recyclable mount ID as the ownership identity.
        statx_mount_id(file, 0x4000)
    }

    #[allow(unsafe_code)]
    fn statx_mount_id(file: &File, mask: u32) -> io::Result<u64> {
        // SAFETY: statx writes into initialized storage and retains no pointer.
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
        // SAFETY: fd and empty pathname are live; open_tree returns an owned mount fd.
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
    fn apply_policy(tree: &File) -> io::Result<()> {
        let attr = MountAttr {
            attr_set: MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV,
            attr_clr: 0,
            propagation: 0,
            userns_fd: 0,
        };
        // SAFETY: mount_attr has the Linux UAPI layout and lives for the syscall.
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
        // SAFETY: fds and empty pathnames are live for the synchronous syscall.
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
        // SAFETY: path is NUL-terminated and live. Flags 0 request normal unmount.
        let result = unsafe { libc::umount2(path.as_ptr(), 0) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{fs, os::unix::ffi::OsStrExt, path::Path};

        fn dir_fd(path: &Path) -> File {
            fs::File::open(path).unwrap()
        }

        #[test]
        fn rejects_invalid_components_before_mount_work() {
            let temp = tempfile::tempdir().unwrap();
            let source = dir_fd(temp.path());
            let parent = dir_fd(temp.path());

            for name in ["", ".", "..", "nested/name"] {
                let error = ManagedExport::prepare(
                    source.try_clone().unwrap(),
                    parent.try_clone().unwrap(),
                    OsStr::new(name),
                )
                .unwrap_err();
                assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
            }
        }

        #[test]
        fn rejects_symlink_target_component() {
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir(temp.path().join("source")).unwrap();
            std::os::unix::fs::symlink("source", temp.path().join("target")).unwrap();

            let error = ManagedExport::prepare(
                dir_fd(&temp.path().join("source")),
                dir_fd(temp.path()),
                OsStr::new("target"),
            )
            .unwrap_err();
            assert!(matches!(
                error.raw_os_error(),
                Some(libc::ENOTDIR | libc::ELOOP)
            ));
        }

        #[test]
        #[ignore = "requires root Linux private mount namespace; run explicitly"]
        fn attaches_and_detaches_in_private_mount_namespace() {
            assert!(
                can_create_private_mount_namespace(),
                "private mount namespace admission failed"
            );
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir(temp.path().join("source")).unwrap();
            fs::create_dir(temp.path().join("target")).unwrap();
            fs::write(temp.path().join("source/file"), b"native").unwrap();

            let target_before = dir_fd(&temp.path().join("target"));
            let covered = directory_identity(&target_before).unwrap();
            let mut export = ManagedExport::prepare(
                dir_fd(&temp.path().join("source")),
                dir_fd(temp.path()),
                OsStr::new("target"),
            )
            .expect("native mount preparation failed");
            export.activate().expect("native mount activation failed");
            assert_eq!(
                export.source_identity(),
                export.mount_identity().unwrap().source
            );
            assert_eq!(
                fs::read(temp.path().join("target/file")).unwrap(),
                b"native"
            );

            export.detach().unwrap();
            let target_after = dir_fd(&temp.path().join("target"));
            assert_eq!(directory_identity(&target_after).unwrap(), covered);
            assert!(!temp.path().join("target/file").exists());
        }

        #[test]
        #[ignore = "requires root Linux private mount namespace; run explicitly"]
        fn detach_rejects_identity_drift_and_retains_claim() {
            assert!(
                can_create_private_mount_namespace(),
                "private mount namespace admission failed"
            );
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir(temp.path().join("source")).unwrap();
            fs::create_dir(temp.path().join("foreign")).unwrap();
            fs::create_dir(temp.path().join("target")).unwrap();

            let mut export = ManagedExport::prepare(
                dir_fd(&temp.path().join("source")),
                dir_fd(temp.path()),
                OsStr::new("target"),
            )
            .expect("native mount preparation failed");
            export.activate().expect("native mount activation failed");
            let claim = export.mount_identity().unwrap();
            bind_mount_private(&temp.path().join("foreign"), &temp.path().join("target")).unwrap();

            let error = export.detach().unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::ESTALE));
            assert_eq!(export.mount_identity().unwrap(), claim);

            normal_unmount(
                &CString::new(temp.path().join("target").as_os_str().as_bytes()).unwrap(),
            )
            .unwrap();
            export.detach().unwrap();
        }

        #[test]
        #[ignore = "requires root Linux private mount namespace; run explicitly"]
        fn detach_of_unactivated_prepare_rejects_foreign_mount() {
            assert!(
                can_create_private_mount_namespace(),
                "private mount namespace admission failed"
            );
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir(temp.path().join("source")).unwrap();
            fs::create_dir(temp.path().join("foreign")).unwrap();
            fs::create_dir(temp.path().join("target")).unwrap();

            let mut export = ManagedExport::prepare(
                dir_fd(&temp.path().join("source")),
                dir_fd(temp.path()),
                OsStr::new("target"),
            )
            .expect("native mount preparation failed");
            bind_mount_private(&temp.path().join("foreign"), &temp.path().join("target")).unwrap();

            let error = export.detach().unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::ESTALE));
            normal_unmount(
                &CString::new(temp.path().join("target").as_os_str().as_bytes()).unwrap(),
            )
            .unwrap();
            export.detach().unwrap();
        }

        #[test]
        #[ignore = "requires root Linux private mount namespace; run explicitly"]
        fn final_clone_detach_rejects_wrong_identity_and_flags_with_proc_hidden() {
            assert!(
                can_create_private_mount_namespace(),
                "private mount namespace admission failed"
            );
            let controller_namespace = File::open("/proc/thread-self/ns/mnt").unwrap();
            let controller_identity = current_namespace().unwrap();
            let controller_cwd = dir_fd(Path::new("."));
            let controller_directory = directory_identity(&controller_cwd).unwrap();
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir(temp.path().join("source")).unwrap();
            fs::create_dir(temp.path().join("root")).unwrap();
            fs::create_dir(temp.path().join("root/workspace")).unwrap();
            fs::write(temp.path().join("source/file"), b"native").unwrap();

            let root = dir_fd(&temp.path().join("root"));
            let covered =
                directory_identity(&open_child(&root, workspace_name()).unwrap()).unwrap();
            let mut export = ManagedExport::prepare(
                dir_fd(&temp.path().join("source")),
                root.try_clone().unwrap(),
                OsStr::new("workspace"),
            )
            .expect("native mount preparation failed");
            export.activate().expect("native mount activation failed");
            let claim = export.mount_identity().unwrap();
            assert_eq!(
                fs::read(temp.path().join("root/workspace/file")).unwrap(),
                b"native"
            );

            assert!(
                can_create_private_mount_namespace(),
                "final clone mount namespace admission failed"
            );
            let final_root = dir_fd(&temp.path().join("root"));
            let final_namespace = File::open("/proc/thread-self/ns/mnt").unwrap();
            hide_proc().unwrap();
            assert_eq!(
                File::open("/proc/thread-self/ns/mnt").unwrap_err().kind(),
                io::ErrorKind::NotFound
            );

            setns_mount(&controller_namespace).expect("restore controller namespace before probe");
            change_directory(&controller_cwd).unwrap();
            let (observed_namespace, observed_source, observed_unique) =
                inspect_final_clone(&final_namespace, &final_root).unwrap();
            assert_ne!(observed_namespace, claim.namespace);
            assert_eq!(observed_source, claim.source);
            assert_ne!(observed_unique, claim.unique_mount_id);
            assert_eq!(current_namespace().unwrap(), controller_identity);

            in_mount_namespace(&final_namespace, || {
                let target = open_child(&final_root, workspace_name())?;
                clear_policy(&target)
            })
            .unwrap();
            assert_eq!(
                inspect_final_clone(&final_namespace, &final_root)
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EPERM)
            );
            assert_eq!(current_namespace().unwrap(), controller_identity);
            assert_eq!(
                detach_final_clone(
                    &final_namespace,
                    &final_root,
                    claim.source,
                    observed_unique,
                    covered
                )
                .unwrap_err()
                .raw_os_error(),
                Some(libc::EPERM)
            );
            assert_eq!(current_namespace().unwrap(), controller_identity);
            in_mount_namespace(&final_namespace, || {
                let target = open_child(&final_root, workspace_name())?;
                apply_policy(&target)
            })
            .unwrap();
            let wrong = DirectoryIdentity {
                device: claim.source.device,
                inode: claim.source.inode.wrapping_add(1),
            };
            let error = detach_final_clone(
                &final_namespace,
                &final_root,
                wrong,
                observed_unique,
                covered,
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::ESTALE));
            assert_eq!(current_namespace().unwrap(), controller_identity);
            let (_, still_source, still_unique) =
                inspect_final_clone(&final_namespace, &final_root).unwrap();
            assert_eq!(still_source, claim.source);
            assert_eq!(still_unique, observed_unique);
            assert_eq!(current_namespace().unwrap(), controller_identity);
            assert_eq!(
                directory_identity(&dir_fd(Path::new("."))).unwrap(),
                controller_directory
            );
            assert_eq!(
                detach_final_clone(
                    &final_namespace,
                    &final_root,
                    claim.source,
                    observed_unique.wrapping_add(1),
                    covered
                )
                .unwrap_err()
                .raw_os_error(),
                Some(libc::ESTALE)
            );
            assert_eq!(current_namespace().unwrap(), controller_identity);
            assert_eq!(
                directory_identity(&dir_fd(Path::new("."))).unwrap(),
                controller_directory
            );

            detach_final_clone(
                &final_namespace,
                &final_root,
                claim.source,
                observed_unique,
                covered,
            )
            .unwrap();
            assert_eq!(current_namespace().unwrap(), controller_identity);
            assert_eq!(
                directory_identity(&dir_fd(Path::new("."))).unwrap(),
                controller_directory
            );
            let (restored_source, restored_unique) = in_mount_namespace(&final_namespace, || {
                let target = open_child(&final_root, workspace_name())?;
                Ok((directory_identity(&target)?, unique_mount_id(&target)?))
            })
            .unwrap();
            assert_eq!(restored_source, covered);
            assert_ne!(restored_unique, observed_unique);
            assert_eq!(
                fs::read(temp.path().join("root/workspace/file")).unwrap(),
                b"native"
            );
            export.detach().unwrap();
            let restored = open_child(&root, workspace_name()).unwrap();
            assert_eq!(directory_identity(&restored).unwrap(), covered);
            assert!(!temp.path().join("root/workspace/file").exists());
        }

        #[allow(unsafe_code)]
        fn hide_proc() -> io::Result<()> {
            // Only this test's cloned private namespace loses its proc view.
            if unsafe {
                libc::mount(
                    std::ptr::null(),
                    c"/proc".as_ptr(),
                    c"tmpfs".as_ptr(),
                    0,
                    std::ptr::null(),
                )
            } == 0
            {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        #[allow(unsafe_code)]
        fn clear_policy(target: &File) -> io::Result<()> {
            let attr = MountAttr {
                attr_set: 0,
                attr_clr: MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV,
                propagation: 0,
                userns_fd: 0,
            };
            // SAFETY: same live descriptor and UAPI layout as production setter;
            // confined to this test's owned final mount namespace.
            if unsafe {
                libc::syscall(
                    libc::SYS_mount_setattr,
                    target.as_raw_fd(),
                    c"".as_ptr(),
                    libc::AT_EMPTY_PATH as u32,
                    &attr as *const MountAttr,
                    std::mem::size_of::<MountAttr>(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[allow(unsafe_code)]
        fn can_create_private_mount_namespace() -> bool {
            unsafe {
                if libc::unshare(libc::CLONE_NEWNS) != 0 {
                    return false;
                }
                let slash = CString::new("/").unwrap();
                libc::mount(
                    std::ptr::null(),
                    slash.as_ptr(),
                    std::ptr::null(),
                    libc::MS_REC | libc::MS_PRIVATE,
                    std::ptr::null(),
                ) == 0
            }
        }

        #[allow(unsafe_code)]
        fn bind_mount_private(source: &Path, target: &Path) -> io::Result<()> {
            let source = CString::new(source.as_os_str().as_bytes()).unwrap();
            let target = CString::new(target.as_os_str().as_bytes()).unwrap();
            let result = unsafe {
                libc::mount(
                    source.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND,
                    std::ptr::null(),
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::{ManagedExport, detach_final_clone, inspect_final_clone};
