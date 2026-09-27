//! OwnerFs-only peer file service. Transport commands enter here in both gRPC-inline
//! and RDMA modes; content movement does not change authorization or POSIX semantics.
//! The default handler intentionally rejects all calls until OwnerFs is wired.
//! A real Home handler can be injected once the Node has a channel-authenticated
//! peer identity source and a local OwnerFs file table.

use std::{
    collections::HashMap,
    ffi::OsString,
    net::SocketAddr,
    os::unix::ffi::OsStringExt,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use afs_protocol::node_data::{
    OwnerCreateReply, OwnerCreateRequest, OwnerDirEntry, OwnerDirectoryHandle, OwnerFlushReply,
    OwnerFlushRequest, OwnerFsyncDirReply, OwnerFsyncDirRequest, OwnerFsyncReply,
    OwnerFsyncRequest, OwnerGetAttrReply, OwnerGetAttrRequest, OwnerLinkReply, OwnerLinkRequest,
    OwnerLookupReply, OwnerLookupRequest, OwnerMkdirReply, OwnerMkdirRequest, OwnerOpenReply,
    OwnerOpenRequest, OwnerOpendirReply, OwnerOpendirRequest, OwnerReadReply, OwnerReadRequest,
    OwnerReaddirReply, OwnerReaddirRequest, OwnerReadlinkReply, OwnerReadlinkRequest,
    OwnerReleaseDirReply, OwnerReleaseDirRequest, OwnerReleaseReply, OwnerReleaseRequest,
    OwnerRenameReply, OwnerRenameRequest, OwnerRmdirReply, OwnerRmdirRequest, OwnerSetAttrReply,
    OwnerSetAttrRequest, OwnerSymlinkReply, OwnerSymlinkRequest, OwnerUnlinkReply,
    OwnerUnlinkRequest, OwnerWriteReply, OwnerWriteRequest,
    owner_files_server::{OwnerFiles, OwnerFilesServer},
};
use afs_transport::grpc::error_status::{coded_status, error_to_status};
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};

use crate::node::vfs::{
    ownerfs::{
        OwnerFsPeerExecutor,
        files::{FileIdentity, RemoteDirectory, RemoteFile},
        root::{PresentedRootAccess, RootId},
    },
    types::{AttributeChange, FileAttributes, FileKind, RenameFlags},
};

/// Home 侧真实文件操作接口。
///
/// 该 trait 保持同步形态，因为 OwnerFs/FUSE 后端目前是同步 VFS 合同，且
/// RootMeta adapter 可能在阻塞线程里等待异步 gRPC。生产多线程 runtime
/// 使用 `block_in_place` 处理短本机操作，使 Tokio 能调度其他任务，同时省去
/// 每次文件 RPC 投递阻塞线程池的开销；单线程测试 runtime 则使用 `spawn_blocking`。
/// `authenticated_peer_node_id` 来自通道认证，不能从请求里的 holder_node_id 复制。
pub trait OwnerFilesHandler: Send + Sync + 'static {
    fn lookup(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLookupRequest,
    ) -> afs_error::Result<OwnerLookupReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Lookup"))
    }
    fn get_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerGetAttrRequest,
    ) -> afs_error::Result<OwnerGetAttrReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.GetAttr"))
    }
    fn set_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSetAttrRequest,
    ) -> afs_error::Result<OwnerSetAttrReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.SetAttr"))
    }
    fn create(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerCreateRequest,
    ) -> afs_error::Result<OwnerCreateReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Create"))
    }
    fn mkdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerMkdirRequest,
    ) -> afs_error::Result<OwnerMkdirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Mkdir"))
    }
    fn unlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerUnlinkRequest,
    ) -> afs_error::Result<OwnerUnlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Unlink"))
    }
    fn rmdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRmdirRequest,
    ) -> afs_error::Result<OwnerRmdirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Rmdir"))
    }
    fn rename(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRenameRequest,
    ) -> afs_error::Result<OwnerRenameReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Rename"))
    }
    fn open(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpenRequest,
    ) -> afs_error::Result<OwnerOpenReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Open"))
    }
    fn readlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadlinkRequest,
    ) -> afs_error::Result<OwnerReadlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Readlink"))
    }
    fn read(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadRequest,
    ) -> afs_error::Result<OwnerReadReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Read"))
    }
    fn write(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerWriteRequest,
    ) -> afs_error::Result<OwnerWriteReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Write"))
    }
    fn flush(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFlushRequest,
    ) -> afs_error::Result<OwnerFlushReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Flush"))
    }
    fn fsync(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncRequest,
    ) -> afs_error::Result<OwnerFsyncReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Fsync"))
    }
    fn release(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseRequest,
    ) -> afs_error::Result<OwnerReleaseReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Release"))
    }
    fn opendir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpendirRequest,
    ) -> afs_error::Result<OwnerOpendirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Opendir"))
    }
    fn readdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReaddirRequest,
    ) -> afs_error::Result<OwnerReaddirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Readdir"))
    }
    fn fsync_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncDirRequest,
    ) -> afs_error::Result<OwnerFsyncDirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.FsyncDir"))
    }
    fn release_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseDirRequest,
    ) -> afs_error::Result<OwnerReleaseDirReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.ReleaseDir"))
    }
    fn symlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSymlinkRequest,
    ) -> afs_error::Result<OwnerSymlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Symlink"))
    }
    fn link(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLinkRequest,
    ) -> afs_error::Result<OwnerLinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Link"))
    }
}

/// Home-side OwnerFiles handler backed by the real OwnerFs peer executor.
///
/// This is the production adapter boundary: gRPC/RDMA command handlers stay in
/// `node/rpc`, while ordinary-file semantics, root-grant validation, stale
/// handle detection, and old-FD behavior stay in `OwnerFsPeerExecutor`.
#[derive(Clone)]
pub struct OwnerFsPeerHandler {
    executor: OwnerFsPeerExecutor,
}

impl OwnerFsPeerHandler {
    #[must_use]
    pub fn new(executor: OwnerFsPeerExecutor) -> Self {
        Self { executor }
    }
}

pub fn make_owner_files_handler(executor: OwnerFsPeerExecutor) -> Arc<dyn OwnerFilesHandler> {
    Arc::new(OwnerFsPeerHandler::new(executor))
}

impl OwnerFilesHandler for OwnerFsPeerHandler {
    fn lookup(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLookupRequest,
    ) -> afs_error::Result<OwnerLookupReply> {
        let access = presented_access(request.access)?;
        let path_is_root = request.path.is_empty();
        let expected_parent = if path_is_root {
            request
                .expected_parent_identity
                .map(|identity| FileIdentity(identity.opaque))
        } else {
            Some(required_identity(
                request.expected_parent_identity,
                "OwnerLookupRequest missing expected_parent_identity",
            )?)
        };
        let entry = self.executor.lookup(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected_parent.as_ref(),
        )?;
        Ok(OwnerLookupReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn get_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerGetAttrRequest,
    ) -> afs_error::Result<OwnerGetAttrReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let remote = request
            .handle
            .map(|handle| remote_file_for_handle(&access, handle.opaque));
        let entry = self.executor.getattr(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            remote.as_ref(),
        )?;
        Ok(OwnerGetAttrReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn create(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerCreateRequest,
    ) -> afs_error::Result<OwnerCreateReply> {
        let access = presented_access(request.access)?;
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerCreateRequest missing expected_parent",
        )?;
        let created = self.executor.create(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            request.flags as i32,
            request.mode,
            &expected_parent,
        )?;
        Ok(OwnerCreateReply {
            handle: Some(afs_protocol::node_data::OwnerHandle {
                opaque: created.file.handle,
            }),
            attr: Some(owner_attr(created.entry.identity, created.entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn mkdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerMkdirRequest,
    ) -> afs_error::Result<OwnerMkdirReply> {
        let access = presented_access(request.access)?;
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerMkdirRequest missing expected_parent",
        )?;
        let entry = self.executor.mkdir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            request.mode,
            &expected_parent,
        )?;
        Ok(OwnerMkdirReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn set_attr(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSetAttrRequest,
    ) -> afs_error::Result<OwnerSetAttrReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let remote = request
            .handle
            .map(|handle| remote_file_for_handle(&access, handle.opaque));
        let change = attribute_change(request.attr)?;
        let entry = self.executor.setattr(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            remote.as_ref(),
            &change,
        )?;
        Ok(OwnerSetAttrReply {
            attr: Some(owner_attr(entry.identity, entry.attributes)),
            owner_session_id: access.home_session_id,
        })
    }

    fn unlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerUnlinkRequest,
    ) -> afs_error::Result<OwnerUnlinkReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerUnlinkRequest missing expected_parent",
        )?;
        self.executor.unlink(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            &expected_parent,
        )?;
        Ok(OwnerUnlinkReply {})
    }

    fn rmdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRmdirRequest,
    ) -> afs_error::Result<OwnerRmdirReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_parent = required_identity(
            request.expected_parent,
            "OwnerRmdirRequest missing expected_parent",
        )?;
        self.executor.rmdir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
            &expected_parent,
        )?;
        Ok(OwnerRmdirReply {})
    }

    fn rename(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerRenameRequest,
    ) -> afs_error::Result<OwnerRenameReply> {
        let access = presented_access(request.access)?;
        let expected_old = request
            .expected_old_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_new = request
            .expected_new_identity
            .map(|identity| FileIdentity(identity.opaque));
        let expected_old_parent = required_identity(
            request.expected_old_parent,
            "OwnerRenameRequest missing expected_old_parent",
        )?;
        let expected_new_parent = required_identity(
            request.expected_new_parent,
            "OwnerRenameRequest missing expected_new_parent",
        )?;
        self.executor.rename(
            authenticated_peer_node_id,
            &access,
            &path_os(request.old_path),
            &path_os(request.new_path),
            expected_old.as_ref(),
            expected_new.as_ref(),
            &expected_old_parent,
            &expected_new_parent,
            RenameFlags(request.flags),
        )?;
        Ok(OwnerRenameReply {})
    }

    fn open(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpenRequest,
    ) -> afs_error::Result<OwnerOpenReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let flags = request.flags as i32;
        let (file, attributes, prefetched_data) = self.executor.open(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            flags,
            expected.as_ref(),
        )?;
        Ok(OwnerOpenReply {
            handle: Some(afs_protocol::node_data::OwnerHandle {
                opaque: file.handle,
            }),
            file_identity: Some(afs_protocol::node_data::FileIdentity {
                opaque: file.identity.0.clone(),
            }),
            owner_session_id: file.owner_session_id,
            attr: Some(owner_attr(file.identity, attributes)),
            prefetched_data,
        })
    }

    fn read(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadRequest,
    ) -> afs_error::Result<OwnerReadReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerReadRequest missing handle")?;
        let mut out = vec![0_u8; request.length as usize];
        let read = self.executor.read(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.offset,
            &mut out,
        )?;
        out.truncate(read);
        Ok(OwnerReadReply {
            data: out,
            read: read as u32,
            eof: read < request.length as usize,
        })
    }

    fn write(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerWriteRequest,
    ) -> afs_error::Result<OwnerWriteReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerWriteRequest missing handle")?;
        let length = request.length as usize;
        if request.data.len() < length {
            return Err(protocol_error("OwnerWriteRequest data shorter than length"));
        }
        let written = self.executor.write(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.offset,
            &request.data[..length],
        )?;
        Ok(OwnerWriteReply {
            written: written as u32,
        })
    }

    fn flush(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFlushRequest,
    ) -> afs_error::Result<OwnerFlushReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerFlushRequest missing handle")?;
        self.executor.flush(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerFlushReply {})
    }

    fn fsync(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncRequest,
    ) -> afs_error::Result<OwnerFsyncReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerFsyncRequest missing handle")?;
        self.executor.fsync(
            authenticated_peer_node_id,
            &access,
            &remote_file_for_handle(&access, handle.opaque),
            request.datasync,
        )?;
        Ok(OwnerFsyncReply {})
    }

    fn release(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseRequest,
    ) -> afs_error::Result<OwnerReleaseReply> {
        let access = presented_access(request.access)?;
        let handle = required_handle(request.handle, "OwnerReleaseRequest missing handle")?;
        self.executor.release(
            authenticated_peer_node_id,
            &access,
            remote_file_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerReleaseReply {})
    }

    fn readlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReadlinkRequest,
    ) -> afs_error::Result<OwnerReadlinkReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let target = self.executor.readlink(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
        )?;
        Ok(OwnerReadlinkReply { target })
    }

    fn opendir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerOpendirRequest,
    ) -> afs_error::Result<OwnerOpendirReply> {
        let access = presented_access(request.access)?;
        let expected = request
            .expected_file_identity
            .map(|identity| FileIdentity(identity.opaque));
        let directory = self.executor.opendir(
            authenticated_peer_node_id,
            &access,
            &path_os(request.path),
            expected.as_ref(),
        )?;
        Ok(OwnerOpendirReply {
            handle: Some(OwnerDirectoryHandle {
                opaque: directory.handle,
            }),
            file_identity: Some(afs_protocol::node_data::FileIdentity {
                opaque: directory.identity.0,
            }),
            owner_session_id: directory.owner_session_id,
        })
    }

    fn readdir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReaddirRequest,
    ) -> afs_error::Result<OwnerReaddirReply> {
        let access = presented_access(request.access)?;
        let max_entries = request.max_entries as usize;
        let handle = required_directory_handle(
            request.handle,
            "OwnerReaddirRequest missing directory handle",
        )?;
        let entries = self.executor.readdir(
            authenticated_peer_node_id,
            &access,
            &remote_directory_for_handle(&access, handle.opaque),
            request.offset,
            max_entries,
        )?;
        let eof = entries.len() < max_entries;
        Ok(OwnerReaddirReply {
            entries: entries
                .into_iter()
                .map(|entry| OwnerDirEntry {
                    name: entry.name.into_vec(),
                    attr: Some(owner_attr(entry.entry.identity, entry.entry.attributes)),
                    next_offset: entry.next_cookie,
                })
                .collect(),
            eof,
        })
    }

    fn fsync_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerFsyncDirRequest,
    ) -> afs_error::Result<OwnerFsyncDirReply> {
        let access = presented_access(request.access)?;
        let handle = required_directory_handle(
            request.handle,
            "OwnerFsyncDirRequest missing directory handle",
        )?;
        self.executor.fsyncdir(
            authenticated_peer_node_id,
            &access,
            &remote_directory_for_handle(&access, handle.opaque),
            request.datasync,
        )?;
        Ok(OwnerFsyncDirReply {})
    }

    fn release_dir(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerReleaseDirRequest,
    ) -> afs_error::Result<OwnerReleaseDirReply> {
        let access = presented_access(request.access)?;
        let handle = required_directory_handle(
            request.handle,
            "OwnerReleaseDirRequest missing directory handle",
        )?;
        self.executor.releasedir(
            authenticated_peer_node_id,
            &access,
            remote_directory_for_handle(&access, handle.opaque),
        )?;
        Ok(OwnerReleaseDirReply {})
    }

    fn symlink(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerSymlinkRequest,
    ) -> afs_error::Result<OwnerSymlinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Symlink"))
    }

    fn link(
        &self,
        authenticated_peer_node_id: &str,
        request: OwnerLinkRequest,
    ) -> afs_error::Result<OwnerLinkReply> {
        let _ = (authenticated_peer_node_id, request);
        Err(owner_handler_unimplemented("OwnerFiles.Link"))
    }
}

fn presented_access(
    access: Option<afs_protocol::node_data::RootAccess>,
) -> afs_error::Result<PresentedRootAccess> {
    let access = access.ok_or_else(|| protocol_error("OwnerFiles request missing RootAccess"))?;
    Ok(PresentedRootAccess {
        id: RootId(access.root_id),
        epoch: access.root_epoch,
        home_node_id: access.home_node_id,
        home_session_id: access.home_session_id,
        holder_node_id: access.holder_node_id,
        session_id: access.session_id,
        access_generation: access.access_generation,
        fencing_token: access.fencing_token,
    })
}

fn path_os(path: Vec<u8>) -> OsString {
    OsString::from_vec(path)
}

fn required_handle(
    handle: Option<afs_protocol::node_data::OwnerHandle>,
    message: &'static str,
) -> afs_error::Result<afs_protocol::node_data::OwnerHandle> {
    handle.ok_or_else(|| protocol_error(message))
}

fn required_directory_handle(
    handle: Option<afs_protocol::node_data::OwnerDirectoryHandle>,
    message: &'static str,
) -> afs_error::Result<afs_protocol::node_data::OwnerDirectoryHandle> {
    handle.ok_or_else(|| protocol_error(message))
}

fn required_identity(
    identity: Option<afs_protocol::node_data::FileIdentity>,
    message: &'static str,
) -> afs_error::Result<FileIdentity> {
    identity
        .map(|identity| FileIdentity(identity.opaque))
        .ok_or_else(|| protocol_error(message))
}

fn attribute_change(
    attr: Option<afs_protocol::node_data::OwnerSetAttr>,
) -> afs_error::Result<AttributeChange> {
    let attr = attr.ok_or_else(|| protocol_error("OwnerSetAttrRequest missing attr"))?;
    Ok(AttributeChange {
        size: attr.size,
        mode: attr.mode,
        uid: attr.uid,
        gid: attr.gid,
        atime: attr.atime_ns.map(ns_to_time),
        mtime: attr.mtime_ns.map(ns_to_time),
    })
}

fn remote_file_for_handle(access: &PresentedRootAccess, handle: Vec<u8>) -> RemoteFile {
    RemoteFile {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity: FileIdentity(Vec::new()),
        handle,
    }
}

fn remote_directory_for_handle(access: &PresentedRootAccess, handle: Vec<u8>) -> RemoteDirectory {
    RemoteDirectory {
        root_id: access.id.clone(),
        owner_node_id: access.home_node_id.clone(),
        owner_session_id: access.home_session_id.clone(),
        identity: FileIdentity(Vec::new()),
        handle,
    }
}

fn ns_to_time(ns: u64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_nanos(ns)
}

fn owner_attr(
    identity: FileIdentity,
    attributes: FileAttributes,
) -> afs_protocol::node_data::OwnerFileAttr {
    afs_protocol::node_data::OwnerFileAttr {
        identity: Some(afs_protocol::node_data::FileIdentity { opaque: identity.0 }),
        kind: owner_kind(attributes.kind).into(),
        mode: attributes.mode,
        uid: attributes.uid,
        gid: attributes.gid,
        size: attributes.size,
        blocks: 0,
        atime_ns: time_ns(attributes.atime),
        mtime_ns: time_ns(attributes.mtime),
        ctime_ns: time_ns(attributes.ctime),
        nlink: attributes.nlink,
        blksize: 4096,
    }
}

fn owner_kind(kind: FileKind) -> afs_protocol::node_data::OwnerFileKind {
    match kind {
        FileKind::Regular => afs_protocol::node_data::OwnerFileKind::Regular,
        FileKind::Directory => afs_protocol::node_data::OwnerFileKind::Directory,
        FileKind::Symlink => afs_protocol::node_data::OwnerFileKind::Symlink,
    }
}

fn time_ns(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn protocol_error(message: &'static str) -> afs_error::Error {
    afs_error::Error::coded(afs_error::CLIENT_PROTOCOL_VIOLATION, message)
}

/// 从已经认证过的 node-to-node 通道提取对端 Node 身份。
///
/// 最终生产实现应绑定 mTLS/SPIFFE SAN、或 Meta 下发的每节点 token 与 TLS
/// channel binding。这里故意不提供“信任 holder_node_id 字段”的实现。
pub trait PeerAuthenticator: Send + Sync + 'static {
    fn authenticate(
        &self,
        metadata: &MetadataMap,
        remote_addr: Option<SocketAddr>,
        peer_cert_der: Option<&[u8]>,
    ) -> afs_error::Result<String>;
}

/// Node-to-node mTLS identity checker backed by an exact certificate allow-list.
///
/// The first production version intentionally avoids parsing X.509 names: Node
/// config provides the verified peer certificate DER for each node id, tonic
/// verifies the TLS client certificate chain, and this authenticator binds the
/// presented leaf certificate bytes to one configured node id. If no certificate
/// is present, or if the bytes are unknown, OwnerFiles fails closed before the
/// request body is inspected.
#[derive(Clone, Debug)]
pub struct MtlsPeerAuthenticator {
    node_id_by_cert_der: Arc<HashMap<Vec<u8>, String>>,
}

impl MtlsPeerAuthenticator {
    pub fn new<I>(trusted_peer_certs: I) -> afs_error::Result<Self>
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        let mut node_id_by_cert_der = HashMap::new();
        for (node_id, cert_der) in trusted_peer_certs {
            if node_id.is_empty() {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    "trusted peer certificate node_id is empty",
                ));
            }
            if cert_der.is_empty() {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    format!("trusted peer certificate for {node_id} is empty"),
                ));
            }
            if let Some(previous) = node_id_by_cert_der.insert(cert_der, node_id.clone()) {
                return Err(afs_error::Error::coded(
                    afs_error::CLIENT_ARGUMENT_INVALID,
                    format!(
                        "the same peer certificate is configured for both {previous} and {node_id}"
                    ),
                ));
            }
        }
        Ok(Self {
            node_id_by_cert_der: Arc::new(node_id_by_cert_der),
        })
    }
}

impl PeerAuthenticator for MtlsPeerAuthenticator {
    fn authenticate(
        &self,
        metadata: &MetadataMap,
        remote_addr: Option<SocketAddr>,
        peer_cert_der: Option<&[u8]>,
    ) -> afs_error::Result<String> {
        let _ = (metadata, remote_addr);
        let Some(peer_cert_der) = peer_cert_der else {
            return Err(afs_error::Error::coded(
                afs_error::NODE_OWNER_INVALID_GRANT,
                "OwnerFiles peer connection has no verified mTLS client certificate",
            ));
        };
        self.node_id_by_cert_der
            .get(peer_cert_der)
            .cloned()
            .ok_or_else(|| {
                afs_error::Error::coded(
                    afs_error::NODE_OWNER_INVALID_GRANT,
                    "OwnerFiles peer certificate is not trusted for any configured node id",
                )
            })
    }
}

/// OwnerFs 远端文件数据面入口。
///
/// 无 handler 的默认实例继续返回 UNIMPLEMENTED，避免把 diagnostics 的 Storage
/// 成功误当成 workspace 文件成功。有 handler 时也必须提供 authenticator；
/// 否则服务 fail-closed，不允许依赖请求体里的 holder_node_id。
#[derive(Clone, Default)]
pub struct OwnerFilesService {
    handler: Option<Arc<dyn OwnerFilesHandler>>,
    authenticator: Option<Arc<dyn PeerAuthenticator>>,
    metrics: Option<super::super::OwnerRpcMetrics>,
}

impl OwnerFilesService {
    #[must_use]
    pub fn new(
        handler: Arc<dyn OwnerFilesHandler>,
        authenticator: Arc<dyn PeerAuthenticator>,
    ) -> Self {
        Self {
            handler: Some(handler),
            authenticator: Some(authenticator),
            metrics: None,
        }
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: super::super::OwnerRpcMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn dispatch<Req, Reply, F>(
        &self,
        request: Request<Req>,
        operation: &'static str,
        call: F,
    ) -> Result<Response<Reply>, Status>
    where
        Req: Send + 'static,
        Reply: Send + 'static,
        F: FnOnce(Arc<dyn OwnerFilesHandler>, String, Req) -> afs_error::Result<Reply>
            + Send
            + 'static,
    {
        let Some(handler) = self.handler.clone() else {
            return Err(owner_files_unimplemented(operation));
        };
        let Some(authenticator) = self.authenticator.clone() else {
            return Err(coded_status(
                afs_error::NODE_OWNER_INVALID_GRANT,
                format!("{operation} has no authenticated peer identity source"),
            ));
        };
        let started = std::time::Instant::now();
        let metadata = request.metadata().clone();
        let remote_addr = request.remote_addr();
        let peer_cert_der = request
            .peer_certs()
            .and_then(|certs| certs.iter().next().map(|cert| cert.as_ref().to_vec()));
        let request = request.into_inner();
        let work = move || {
            let authenticated_peer_node_id =
                authenticator.authenticate(&metadata, remote_addr, peer_cert_der.as_deref())?;
            call(handler, authenticated_peer_node_id, request)
        };
        // Production Node uses a multi-thread runtime. Run the short local
        // file action on this worker and let Tokio hand other tasks off while
        // it blocks, avoiding a blocking-pool scheduling hop per file RPC.
        // Current-thread test runtimes still need spawn_blocking.
        let result = if tokio::runtime::Handle::current().runtime_flavor()
            == tokio::runtime::RuntimeFlavor::MultiThread
        {
            Ok(tokio::task::block_in_place(work))
        } else {
            tokio::task::spawn_blocking(work).await
        };
        if let Some(metrics) = &self.metrics {
            metrics.observe("server", operation, started.elapsed());
        }
        let reply = result
            .map_err(|error| {
                coded_status(
                    afs_error::CLIENT_WORKER_FAILED,
                    format!("{operation} blocking worker failed: {error}"),
                )
            })?
            .map_err(error_to_status)?;
        Ok(Response::new(reply))
    }
}

#[must_use]
pub fn make_owner_files_server() -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::default())
}

#[must_use]
pub fn make_owner_files_server_with_handler(
    handler: Arc<dyn OwnerFilesHandler>,
    authenticator: Arc<dyn PeerAuthenticator>,
) -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::new(handler, authenticator))
}

#[must_use]
pub fn make_owner_files_server_with_handler_and_metrics(
    handler: Arc<dyn OwnerFilesHandler>,
    authenticator: Arc<dyn PeerAuthenticator>,
    metrics: super::super::OwnerRpcMetrics,
) -> OwnerFilesServer<OwnerFilesService> {
    OwnerFilesServer::new(OwnerFilesService::new(handler, authenticator).with_metrics(metrics))
}

#[tonic::async_trait]
impl OwnerFiles for OwnerFilesService {
    async fn lookup(
        &self,
        request: Request<OwnerLookupRequest>,
    ) -> Result<Response<OwnerLookupReply>, Status> {
        self.dispatch(request, "OwnerFiles.Lookup", |handler, peer, request| {
            handler.lookup(&peer, request)
        })
        .await
    }

    async fn get_attr(
        &self,
        request: Request<OwnerGetAttrRequest>,
    ) -> Result<Response<OwnerGetAttrReply>, Status> {
        self.dispatch(request, "OwnerFiles.GetAttr", |handler, peer, request| {
            handler.get_attr(&peer, request)
        })
        .await
    }

    async fn set_attr(
        &self,
        request: Request<OwnerSetAttrRequest>,
    ) -> Result<Response<OwnerSetAttrReply>, Status> {
        self.dispatch(request, "OwnerFiles.SetAttr", |handler, peer, request| {
            handler.set_attr(&peer, request)
        })
        .await
    }

    async fn create(
        &self,
        request: Request<OwnerCreateRequest>,
    ) -> Result<Response<OwnerCreateReply>, Status> {
        self.dispatch(request, "OwnerFiles.Create", |handler, peer, request| {
            handler.create(&peer, request)
        })
        .await
    }

    async fn mkdir(
        &self,
        request: Request<OwnerMkdirRequest>,
    ) -> Result<Response<OwnerMkdirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Mkdir", |handler, peer, request| {
            handler.mkdir(&peer, request)
        })
        .await
    }

    async fn unlink(
        &self,
        request: Request<OwnerUnlinkRequest>,
    ) -> Result<Response<OwnerUnlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Unlink", |handler, peer, request| {
            handler.unlink(&peer, request)
        })
        .await
    }

    async fn rmdir(
        &self,
        request: Request<OwnerRmdirRequest>,
    ) -> Result<Response<OwnerRmdirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Rmdir", |handler, peer, request| {
            handler.rmdir(&peer, request)
        })
        .await
    }

    async fn rename(
        &self,
        request: Request<OwnerRenameRequest>,
    ) -> Result<Response<OwnerRenameReply>, Status> {
        self.dispatch(request, "OwnerFiles.Rename", |handler, peer, request| {
            handler.rename(&peer, request)
        })
        .await
    }

    async fn open(
        &self,
        request: Request<OwnerOpenRequest>,
    ) -> Result<Response<OwnerOpenReply>, Status> {
        self.dispatch(request, "OwnerFiles.Open", |handler, peer, request| {
            handler.open(&peer, request)
        })
        .await
    }

    async fn readlink(
        &self,
        request: Request<OwnerReadlinkRequest>,
    ) -> Result<Response<OwnerReadlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Readlink", |handler, peer, request| {
            handler.readlink(&peer, request)
        })
        .await
    }

    async fn read(
        &self,
        request: Request<OwnerReadRequest>,
    ) -> Result<Response<OwnerReadReply>, Status> {
        self.dispatch(request, "OwnerFiles.Read", |handler, peer, request| {
            handler.read(&peer, request)
        })
        .await
    }

    async fn write(
        &self,
        request: Request<OwnerWriteRequest>,
    ) -> Result<Response<OwnerWriteReply>, Status> {
        self.dispatch(request, "OwnerFiles.Write", |handler, peer, request| {
            handler.write(&peer, request)
        })
        .await
    }

    async fn flush(
        &self,
        request: Request<OwnerFlushRequest>,
    ) -> Result<Response<OwnerFlushReply>, Status> {
        self.dispatch(request, "OwnerFiles.Flush", |handler, peer, request| {
            handler.flush(&peer, request)
        })
        .await
    }

    async fn fsync(
        &self,
        request: Request<OwnerFsyncRequest>,
    ) -> Result<Response<OwnerFsyncReply>, Status> {
        self.dispatch(request, "OwnerFiles.Fsync", |handler, peer, request| {
            handler.fsync(&peer, request)
        })
        .await
    }

    async fn release(
        &self,
        request: Request<OwnerReleaseRequest>,
    ) -> Result<Response<OwnerReleaseReply>, Status> {
        self.dispatch(request, "OwnerFiles.Release", |handler, peer, request| {
            handler.release(&peer, request)
        })
        .await
    }

    async fn opendir(
        &self,
        request: Request<OwnerOpendirRequest>,
    ) -> Result<Response<OwnerOpendirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Opendir", |handler, peer, request| {
            handler.opendir(&peer, request)
        })
        .await
    }

    async fn readdir(
        &self,
        request: Request<OwnerReaddirRequest>,
    ) -> Result<Response<OwnerReaddirReply>, Status> {
        self.dispatch(request, "OwnerFiles.Readdir", |handler, peer, request| {
            handler.readdir(&peer, request)
        })
        .await
    }

    async fn fsync_dir(
        &self,
        request: Request<OwnerFsyncDirRequest>,
    ) -> Result<Response<OwnerFsyncDirReply>, Status> {
        self.dispatch(request, "OwnerFiles.FsyncDir", |handler, peer, request| {
            handler.fsync_dir(&peer, request)
        })
        .await
    }

    async fn release_dir(
        &self,
        request: Request<OwnerReleaseDirRequest>,
    ) -> Result<Response<OwnerReleaseDirReply>, Status> {
        self.dispatch(
            request,
            "OwnerFiles.ReleaseDir",
            |handler, peer, request| handler.release_dir(&peer, request),
        )
        .await
    }

    async fn symlink(
        &self,
        request: Request<OwnerSymlinkRequest>,
    ) -> Result<Response<OwnerSymlinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Symlink", |handler, peer, request| {
            handler.symlink(&peer, request)
        })
        .await
    }

    async fn link(
        &self,
        request: Request<OwnerLinkRequest>,
    ) -> Result<Response<OwnerLinkReply>, Status> {
        self.dispatch(request, "OwnerFiles.Link", |handler, peer, request| {
            handler.link(&peer, request)
        })
        .await
    }
}

fn owner_files_unimplemented(operation: &'static str) -> Status {
    coded_status(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("{operation} is not wired to OwnerFs yet"),
    )
}

fn owner_handler_unimplemented(operation: &'static str) -> afs_error::Error {
    afs_error::Error::coded(
        afs_error::NODE_VFS_UNIMPLEMENTED,
        format!("{operation} is not implemented by the injected OwnerFs handler"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtls_authenticator_binds_peer_identity_to_exact_leaf_der() {
        let authenticator = MtlsPeerAuthenticator::new(vec![("node-b".to_owned(), vec![1, 2, 3])])
            .expect("trusted cert config should be valid");
        let authenticated = authenticator
            .authenticate(&MetadataMap::new(), None, Some(&[1, 2, 3]))
            .expect("known cert should authenticate");
        assert_eq!(authenticated, "node-b");
    }

    #[test]
    fn mtls_authenticator_fails_closed_without_verified_cert() {
        let authenticator = MtlsPeerAuthenticator::new(vec![("node-b".to_owned(), vec![1, 2, 3])])
            .expect("trusted cert config should be valid");
        let error = authenticator
            .authenticate(&MetadataMap::new(), None, None)
            .expect_err("OwnerFiles must not accept unauthenticated peers");
        assert_eq!(error.code(), afs_error::NODE_OWNER_INVALID_GRANT);
    }

    #[test]
    fn mtls_authenticator_rejects_ambiguous_cert_config() {
        let error = MtlsPeerAuthenticator::new(vec![
            ("node-a".to_owned(), vec![9, 9]),
            ("node-b".to_owned(), vec![9, 9]),
        ])
        .expect_err("one certificate cannot identify two nodes");
        assert_eq!(error.code(), afs_error::CLIENT_ARGUMENT_INVALID);
    }
}
