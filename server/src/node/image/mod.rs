//! 镜像类懒加载读取的领域入口。
//!
//! `open_layer` 固定当前对象版本；后续 `read_at` 都按该版本读取 range。这样镜像层
//! 的分层、分块和摘要语义只存在于本模块，DataCore 仍只看对象版本和 byte range。

use super::data_core::{
    ByteRange, DataCoreHandle, ObjectKey, ObjectRead, ReadOptions, VersionSelector,
};
use super::runtime::WorkerError;

#[derive(Clone)]
#[allow(
    dead_code,
    reason = "本轮固化 Image lazy-range 边界，不实现真实镜像挂载"
)]
pub(crate) struct ImageReader {
    core: DataCoreHandle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code, reason = "由尚未挂载的 Image 读取合同持有")]
pub(crate) struct ImageLayer {
    key: ObjectKey,
    version: u64,
    length: u64,
}

#[allow(
    dead_code,
    reason = "本轮固化 Image lazy-range 边界，不实现真实镜像挂载"
)]
impl ImageReader {
    pub(crate) fn new(core: DataCoreHandle) -> Self {
        Self { core }
    }

    pub(crate) async fn open_layer(
        &self,
        object: impl Into<Vec<u8>>,
    ) -> Result<Option<ImageLayer>, WorkerError> {
        let key = ObjectKey::new(object)?;
        let Some(stat) = self.core.stat(key.clone()).await? else {
            return Ok(None);
        };
        Ok(Some(ImageLayer {
            key,
            version: stat.version,
            length: stat.length,
        }))
    }

    pub(crate) async fn read_at(
        &self,
        layer: &ImageLayer,
        offset: u64,
        length: u64,
    ) -> Result<Option<ObjectRead>, WorkerError> {
        self.core
            .read(
                layer.key.clone(),
                ReadOptions {
                    version: VersionSelector::Exact(layer.version),
                    range: Some(ByteRange::new(offset, length)?),
                    clamp_range: true,
                },
            )
            .await
    }

    pub(crate) async fn read_exact_at(
        &self,
        layer: &ImageLayer,
        offset: u64,
        output: &mut [u8],
    ) -> Result<Option<usize>, WorkerError> {
        let Some(meta) = self
            .core
            .read_into(
                layer.key.clone(),
                ReadOptions {
                    version: VersionSelector::Exact(layer.version),
                    range: Some(ByteRange::new(offset, output.len() as u64)?),
                    clamp_range: false,
                },
                output,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(meta.bytes_read))
    }

    pub(crate) fn layer_length(layer: &ImageLayer) -> u64 {
        layer.length
    }
}
