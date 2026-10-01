//! Trusted Home authority and pinned backing for physical export admission.
//!
//! A permit is a revalidated snapshot, not a lease for future native access.
//! Agent admission, continuous fencing and lifecycle drainage are separate.
use super::{DirectoryIdentity, LinuxMountBackend, NamespaceIdentity, WorkspaceIdentity};
use crate::node::vfs::{
    Backend,
    types::{BackendInode, Entry},
};
use crate::node::{
    storage::{FileStore, StoragePath},
    vfs::ownerfs::{
        LocalOwnerFs, OwnerFs, attributes_from_metadata,
        root::{RootGrant, RootRight, root_id_from_name},
    },
};
use afs_error::{Error, Result};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    sync::{Arc, Weak},
};

/// Only OwnerFs can issue this permit from its current trusted Home grant.
/// Keep the exact directory open; a matching pathname is not object identity.
pub struct HomeExportAuthority {
    owner: Weak<LocalOwnerFs>,
    grant: RootGrant,
    identity: WorkspaceIdentity,
    name: OsString,
    data_dir: StoragePath,
    source: File,
    source_identity: DirectoryIdentity,
    _anchor: Arc<RootAnchor>,
}

/// Only namespace metadata, never a data-operation or peer grant. This pins
/// the mountpoint's positive FUSE identity until normal teardown has finished.
pub(in crate::node::vfs::ownerfs) struct RootAnchor {
    identity: WorkspaceIdentity,
    source_identity: DirectoryIdentity,
    inode: BackendInode,
    source: File,
}

impl RootAnchor {
    fn entry(&self) -> Result<Entry> {
        if directory_identity(&self.source)? != self.source_identity {
            return Err(errno(libc::ESTALE));
        }
        Ok(Entry {
            inode: self.inode,
            attributes: attributes_from_metadata(self.source.metadata()?)?,
        })
    }
}

impl OwnerFs {
    /// Snapshot Home export authority without mounting or acknowledging READY.
    /// Cache eligibility must have been selected before the first FUSE reply.
    pub fn native_home_export(
        &self,
        name: &OsStr,
        namespace: NamespaceIdentity,
    ) -> Result<HomeExportAuthority> {
        check_eligible(self, namespace)?;
        if name.as_bytes().len() > 255 {
            return Err(errno(libc::ENAMETOOLONG));
        }
        let root_id = root_id_from_name(name)?;
        let local = self.require_local()?;
        let admitted = local.roots.enter_root(&root_id, RootRight::Write)?;
        let grant = admitted.grant().clone();
        check_home_rights(&grant)?;
        let data_dir = admitted.data_dir().clone();
        let source = local.disk.open_dir(&data_dir)?.try_clone_descriptor()?;
        let source_identity = directory_identity(&source)?;
        // Do not retain an in-flight RootUse across mount transactions: that
        // would prevent the lifecycle controller from completing drainage.
        drop(admitted);
        let identity = WorkspaceIdentity {
            root_id: grant.id.0.clone(),
            epoch: grant.epoch,
            home_node_id: grant.home_node_id.clone(),
            home_session_id: grant.home_session_id.clone(),
            namespace,
        };
        let entry = local.lookup(self.root_inode(), name)?;
        let mut cache = self.private_cache.lock().map_err(|_| errno(libc::EIO))?;
        cache
            .native_roots
            .retain(|_, anchor| anchor.strong_count() != 0);
        let anchor = if let Some(old) = cache.native_roots.get(&grant.id).and_then(Weak::upgrade) {
            if old.identity.root_id != identity.root_id
                || old.identity.epoch != identity.epoch
                || old.identity.home_node_id != identity.home_node_id
                || old.identity.home_session_id != identity.home_session_id
                || old.source_identity != source_identity
                || old.inode != entry.inode
            {
                return Err(errno(libc::EBUSY));
            }
            old
        } else {
            if cache.native_roots.len() >= 4096 {
                return Err(errno(libc::ENOSPC));
            }
            let anchor = Arc::new(RootAnchor {
                identity: identity.clone(),
                source_identity,
                inode: entry.inode,
                source: source.try_clone()?,
            });
            cache
                .native_roots
                .insert(grant.id.clone(), Arc::downgrade(&anchor));
            anchor
        };
        drop(cache);
        let permit = HomeExportAuthority {
            owner: Arc::downgrade(self.local.as_ref().ok_or_else(|| errno(libc::ESTALE))?),
            grant,
            identity,
            name: name.to_os_string(),
            data_dir,
            source,
            source_identity,
            _anchor: anchor,
        };
        permit.verify_current(self)?;
        Ok(permit)
    }

    pub(in crate::node::vfs::ownerfs) fn native_root_entry(
        &self,
        name: &OsStr,
    ) -> Result<Option<Entry>> {
        let id = root_id_from_name(name)?;
        let anchor = self
            .private_cache
            .lock()
            .map_err(|_| errno(libc::EIO))?
            .native_roots
            .get(&id)
            .and_then(Weak::upgrade);
        anchor.map(|anchor| anchor.entry()).transpose()
    }

    pub(in crate::node::vfs::ownerfs) fn native_root_attributes(
        &self,
        inode: BackendInode,
    ) -> Result<Option<Entry>> {
        let anchor = self
            .private_cache
            .lock()
            .map_err(|_| errno(libc::EIO))?
            .native_roots
            .values()
            .filter_map(Weak::upgrade)
            .find(|anchor| anchor.inode == inode);
        anchor.map(|anchor| anchor.entry()).transpose()
    }
}

impl HomeExportAuthority {
    #[must_use]
    pub fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    #[must_use]
    pub fn source_identity(&self) -> DirectoryIdentity {
        self.source_identity
    }

    pub(super) fn source_descriptor(&self) -> io::Result<File> {
        self.source.try_clone()
    }

    pub(super) fn name(&self) -> &OsStr {
        &self.name
    }

    /// Check authority, namespace and the current confined backing directory.
    /// Even a grant with the same Root/epoch is stale if its fencing generation,
    /// rights, process session or directory object changed.
    pub fn verify_current(&self, owner: &OwnerFs) -> Result<()> {
        check_eligible(owner, self.identity.namespace)?;
        self.verify_owner(owner)?;
        let current = owner.local.as_ref().ok_or_else(|| errno(libc::ESTALE))?;
        if root_id_from_name(&self.name)? != self.grant.id {
            return Err(errno(libc::ESTALE));
        }
        let admitted = current.roots.enter_root(&self.grant.id, RootRight::Write)?;
        check_home_rights(admitted.grant())?;
        if admitted.grant() != &self.grant || admitted.data_dir() != &self.data_dir {
            return Err(errno(libc::ESTALE));
        }
        let source = current
            .disk
            .open_dir(admitted.data_dir())?
            .try_clone_descriptor()?;
        if directory_identity(&source)? != self.source_identity
            || directory_identity(&self.source)? != self.source_identity
        {
            return Err(errno(libc::ESTALE));
        }
        Ok(())
    }

    /// Prove the caller belongs to the issuing instance before any mutation,
    /// including cleanup. A foreign caller must not quiesce a valid export.
    pub(super) fn verify_owner(&self, owner: &OwnerFs) -> Result<()> {
        let original = self.owner.upgrade().ok_or_else(|| errno(libc::ESTALE))?;
        let current = owner.local.as_ref().ok_or_else(|| errno(libc::ESTALE))?;
        if !Arc::ptr_eq(&original, current) {
            return Err(errno(libc::ESTALE));
        }
        Ok(())
    }
}

fn check_eligible(owner: &OwnerFs, namespace: NamespaceIdentity) -> Result<()> {
    let eligible = owner
        .private_cache
        .lock()
        .map_err(|_| errno(libc::EIO))?
        .native_eligible;
    if !eligible {
        return Err(errno(libc::EPERM));
    }
    if LinuxMountBackend::current_namespace()? != namespace {
        return Err(errno(libc::ESTALE));
    }
    Ok(())
}

fn check_home_rights(grant: &RootGrant) -> Result<()> {
    if grant.holder_node_id != grant.home_node_id
        || grant.session_id != grant.home_session_id
        || ![RootRight::Lookup, RootRight::Read, RootRight::Write]
            .iter()
            .all(|right| grant.rights.contains(right))
    {
        return Err(errno(libc::EACCES));
    }
    Ok(())
}

fn directory_identity(file: &File) -> io::Result<DirectoryIdentity> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
    }
    Ok(DirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn errno(code: i32) -> Error {
    io::Error::from_raw_os_error(code).into()
}
