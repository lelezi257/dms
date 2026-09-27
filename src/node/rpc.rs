//! Node 内部通信边界。
//!
//! control/data 接收其他 Node 请求；peer 调用其他 Node；meta 调用中心。
//! 控制统一 gRPC，文件内容可用 gRPC 或单边 RDMA。通道选择不改变业务成功合同。
//! 本模块理解文件操作，公共 transport 只处理传输机制；业务与资源锁归业务所有者。

pub mod control;
pub mod data;
pub mod meta;
pub mod peer;

#[cfg(feature = "ownerfs")]
use afs_metrics::{HistogramOpts, HistogramVec, MetricsError, Registry, register_collector};

/// Bounded per-method timing of OwnerFiles RPCs. The same registry is exposed
/// by the Node /metrics endpoint, so a W2 phase can compare the B-side round
/// trip with A-side handler work without turning on per-request log output.
#[cfg(feature = "ownerfs")]
#[derive(Clone)]
pub struct OwnerRpcMetrics {
    duration_seconds: HistogramVec,
}

#[cfg(feature = "ownerfs")]
impl OwnerRpcMetrics {
    pub fn register(registry: &Registry) -> Result<Self, MetricsError> {
        registry.get_or_register(|registry| {
            let duration_seconds = HistogramVec::new(
                HistogramOpts::new(
                    "afs_ownerfiles_rpc_duration_seconds",
                    "OwnerFiles client round trip and Home handler duration by method.",
                )
                .buckets(vec![
                    0.000_05, 0.000_1, 0.000_2, 0.000_4, 0.000_8, 0.001_6, 0.003_2, 0.006_4,
                    0.012_8,
                ]),
                &["side", "method"],
            )?;
            register_collector(registry, &duration_seconds)?;
            Ok(Self { duration_seconds })
        })
    }

    pub fn observe(&self, side: &'static str, method: &'static str, elapsed: std::time::Duration) {
        self.duration_seconds
            .with_label_values(&[side, method])
            .observe(elapsed.as_secs_f64());
    }
}
