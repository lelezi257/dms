//! Node 内置 Filesystem 到 Meta 的独立 gRPC handler。
//!
//! generated service 只负责 wire 边界；所有 namespace、CAS、Journal 与 Watch 状态仍
//! 由同一个 `MetaHandle` 投递到唯一 `MetaState`。

use dms_protocol::v1 as pb;
use dms_transport::dms_error_to_status;
use pb::filesystem_metadata_service_server::FilesystemMetadataService;
use tonic::{Request, Response, Status};

use crate::meta::runtime::{MetaHandle, MetaRuntimeError};

#[derive(Clone)]
pub(crate) struct FilesystemMetadataServiceHandler {
    meta: MetaHandle,
    rpc_metrics: dms_metrics::RpcMetrics,
    error_metrics: dms_metrics::ErrorMetrics,
}

impl FilesystemMetadataServiceHandler {
    pub(crate) fn new(
        meta: MetaHandle,
        rpc_metrics: dms_metrics::RpcMetrics,
        error_metrics: dms_metrics::ErrorMetrics,
    ) -> Self {
        Self {
            meta,
            rpc_metrics,
            error_metrics,
        }
    }

    fn map_error(&self, error: MetaRuntimeError) -> Status {
        let error = error.into_dms_error();
        self.error_metrics
            .record_if_component(dms_metrics::ErrorComponent::Meta, &error);
        dms_error_to_status(error)
    }
}

#[tonic::async_trait]
impl FilesystemMetadataService for FilesystemMetadataServiceHandler {
    async fn lookup_filesystem_entry(
        &self,
        request: Request<pb::FilesystemLookupRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_LOOKUP);
        let response = self
            .meta
            .filesystem_lookup(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn get_filesystem_inode(
        &self,
        request: Request<pb::FilesystemGetInodeRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_GET_INODE);
        let response = self
            .meta
            .filesystem_get_inode(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn create_filesystem_inode(
        &self,
        request: Request<pb::FilesystemCreateInodeRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_CREATE_INODE);
        let response = self
            .meta
            .filesystem_create_inode(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn create_filesystem_symlink(
        &self,
        request: Request<pb::FilesystemCreateSymlinkRequest>,
    ) -> Result<Response<pb::FilesystemResolveResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_CREATE_SYMLINK);
        let response = self
            .meta
            .filesystem_create_symlink(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn read_filesystem_directory(
        &self,
        request: Request<pb::FilesystemReadDirectoryRequest>,
    ) -> Result<Response<pb::FilesystemReadDirectoryResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_READ_DIRECTORY);
        let response = self
            .meta
            .filesystem_read_directory(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn link_filesystem_entry(
        &self,
        request: Request<pb::FilesystemLinkRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_LINK);
        let response = self
            .meta
            .filesystem_link_entry(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn acquire_filesystem_inode_reference(
        &self,
        request: Request<pb::FilesystemInodeReferenceRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_ACQUIRE_INODE_REFERENCE);
        let response = self
            .meta
            .filesystem_acquire_inode_reference(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn release_filesystem_inode_reference(
        &self,
        request: Request<pb::FilesystemInodeReferenceRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_RELEASE_INODE_REFERENCE);
        let response = self
            .meta
            .filesystem_release_inode_reference(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn renew_filesystem_inode_references(
        &self,
        request: Request<pb::FilesystemRenewInodeReferencesRequest>,
    ) -> Result<Response<pb::FilesystemInodeReferenceResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_RENEW_INODE_REFERENCES);
        let response = self
            .meta
            .filesystem_renew_inode_references(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn rename_filesystem_entry(
        &self,
        request: Request<pb::FilesystemRenameRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_RENAME);
        let response = self
            .meta
            .filesystem_rename_entry(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn remove_filesystem_entry(
        &self,
        request: Request<pb::FilesystemRemoveRequest>,
    ) -> Result<Response<pb::FilesystemNamespaceMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_REMOVE);
        let response = self
            .meta
            .filesystem_remove_entry(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn set_filesystem_attributes(
        &self,
        request: Request<pb::FilesystemSetAttributesRequest>,
    ) -> Result<Response<pb::FilesystemAttributeMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_SET_ATTRIBUTES);
        let response = self
            .meta
            .filesystem_set_attributes(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn get_filesystem_xattr(
        &self,
        request: Request<pb::FilesystemGetXattrRequest>,
    ) -> Result<Response<pb::FilesystemGetXattrResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_GET_XATTR);
        let response = self
            .meta
            .filesystem_get_xattr(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn list_filesystem_xattrs(
        &self,
        request: Request<pb::FilesystemListXattrsRequest>,
    ) -> Result<Response<pb::FilesystemListXattrsResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_LIST_XATTRS);
        let response = self
            .meta
            .filesystem_list_xattrs(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn set_filesystem_xattr(
        &self,
        request: Request<pb::FilesystemSetXattrRequest>,
    ) -> Result<Response<pb::FilesystemAttributeMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_SET_XATTR);
        let response = self
            .meta
            .filesystem_set_xattr(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn remove_filesystem_xattr(
        &self,
        request: Request<pb::FilesystemRemoveXattrRequest>,
    ) -> Result<Response<pb::FilesystemAttributeMutationResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_REMOVE_XATTR);
        let response = self
            .meta
            .filesystem_remove_xattr(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn stat_filesystem(
        &self,
        request: Request<pb::FilesystemStatRequest>,
    ) -> Result<Response<pb::FilesystemStatResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_STAT);
        let response = self
            .meta
            .filesystem_stat(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }

    async fn commit_filesystem_version(
        &self,
        request: Request<pb::FilesystemCommitVersionRequest>,
    ) -> Result<Response<pb::FilesystemCommitVersionResponse>, Status> {
        let mut rpc = self
            .rpc_metrics
            .begin_server_call(dms_metrics::RpcCall::FILESYSTEM_COMMIT_VERSION);
        let response = self
            .meta
            .filesystem_commit_version(request.into_inner())
            .await
            .map_err(|error| self.map_error(error))?;
        rpc.success();
        Ok(Response::new(response))
    }
}
