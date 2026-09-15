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
