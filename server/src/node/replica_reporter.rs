//! Peer 接管完成后的异步副本登记。
//!
//! Node 先把远端 Block 拉到本地、校验并安装进 Arena，再把一条不含 payload
//! bytes 的 [`ReplicaReportJob`] 放入有界队列。正常 GET 只等待入队，不等待
//! Meta WAL；本模块串行消费队列、合并同一波报告并以固定幂等身份重试。
//!
//! 这里没有 Node 业务状态，也不拥有 Block。`NodeState` 仍是数据与生命周期的
//! 唯一写 owner；Reporter 只是把“本 Node 已可读取该 Block”这个事实登记到 Meta。

use std::time::Duration;

use dms_error::DmsError;
use dms_protocol::v1 as pb;
use tokio::sync::mpsc;

use super::metadata_client::{MetadataClient, digest};

const RETRY_INITIAL: Duration = Duration::from_millis(10);
const RETRY_MAX: Duration = Duration::from_secs(1);
// 副本登记不在用户 GET 的前台路径上，可以用短窗口合并同一波接管报告。
const COALESCE_WINDOW: Duration = Duration::from_millis(5);
const BATCH_MAX: usize = 64;

/// 一个已经完成本地安装、等待向 Meta 登记的不可变 Block。
///
/// 任务只携带可重试的控制信息，不复制 payload，也不持有用户读票据。
#[derive(Clone, Debug)]
pub(crate) struct ReplicaReportJob {
    block_id: Vec<u8>,
    length: u64,
    checksum: Vec<u8>,
    operation_id: Vec<u8>,
}

impl ReplicaReportJob {
    pub(crate) fn new(
        block_id: Vec<u8>,
        length: u64,
        checksum: Vec<u8>,
        operation_id: Vec<u8>,
    ) -> Self {
        Self {
            block_id,
            length,
            checksum,
            operation_id,
        }
    }
}

/// 一次 Meta 请求中的报告集合。
///
/// 批内成员与 `operation_id` 在重试期间保持不变，因此响应丢失后重发仍会被
/// Meta 幂等表识别为同一次操作。
#[derive(Clone, Debug)]
struct ReplicaReportBatch {
    jobs: Vec<ReplicaReportJob>,
    operation_id: Vec<u8>,
}

impl ReplicaReportBatch {
    fn new(jobs: Vec<ReplicaReportJob>) -> Self {
        debug_assert!(!jobs.is_empty());
        let mut identity = b"dms:replica-report-batch:v1".to_vec();
        for job in &jobs {
            // 长度前缀避免不同 operation_id 序列简单拼接后产生相同字节串。
            identity.extend_from_slice(&(job.operation_id.len() as u64).to_be_bytes());
            identity.extend_from_slice(&job.operation_id);
        }
        Self {
            operation_id: digest(&identity),
            jobs,
        }
    }
}

/// 串行消费有界队列；Meta 故障时不会为每次重试创建无限后台 Task。
///
/// 第一项到达后最多等待 5 ms，吸收最多 64 个已经到达的报告。这个等待只影响
/// 其它节点何时发现本副本，不延长发起接管的用户 GET；队列满时生产者会受到
/// 明确背压。Meta 已退休的 Block 会返回终态拒绝，迟到任务不会复活旧副本。
pub(crate) async fn run(metadata: MetadataClient, mut jobs: mpsc::Receiver<ReplicaReportJob>) {
    while let Some(first) = jobs.recv().await {
        let batch = collect_batch(first, &mut jobs).await;
        let mut retry_delay = RETRY_INITIAL;
        let mut failures = 0_u64;
        loop {
            match report_batch(&metadata, &batch).await {
                Ok(()) => break,
                Err(error) => {
                    failures += 1;
                    // 只记录首失败和 2 的幂次，避免持续故障淹没日志；RPC
                    // Metrics 仍逐次记录结果和耗时。
                    if failures.is_power_of_two() {
                        dms_logging::warn!(
                            "background replica report failed; retrying";
                            "event" => "node.replica_report.retry",
                            "attempt" => failures,
                            "error" => format!("{error:?}"),
                        );
                    }
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(RETRY_MAX);
                }
            }
        }
    }
}

/// 不经过后台队列立即登记一项。
///
/// 仅供需要精确观察 Meta 错误的直接完整性测试路径复用；正常 Peer GET 使用
/// [`run`] 的有界后台队列。
pub(crate) async fn report_now(
    metadata: &MetadataClient,
    job: ReplicaReportJob,
) -> Result<(), DmsError> {
    metadata
        .report_replica(
            job.block_id,
            job.length,
            job.checksum,
            job.operation_id,
            2,
            Vec::new(),
        )
        .await
}

/// 等待固定后台窗口后，仅吸收此刻已经排队的项；`try_recv` 不会继续挂起凑批。
async fn collect_batch(
    first: ReplicaReportJob,
    jobs: &mut mpsc::Receiver<ReplicaReportJob>,
) -> ReplicaReportBatch {
    tokio::time::sleep(COALESCE_WINDOW).await;
    let mut batch = Vec::with_capacity(BATCH_MAX);
    batch.push(first);
    while batch.len() < BATCH_MAX {
        match jobs.try_recv() {
            Ok(job) => batch.push(job),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    ReplicaReportBatch::new(batch)
}

async fn report_batch(
    metadata: &MetadataClient,
    batch: &ReplicaReportBatch,
) -> Result<(), DmsError> {
    metadata
        .report_replicas(
            batch
                .jobs
                .iter()
                .map(|job| pb::ReplicaReport {
                    block_id: job.block_id.clone(),
                    length: job.length,
                    checksum: job.checksum.clone(),
                    durability: pb::DurabilityPolicy::ReplicatedMemory as i32,
                })
                .collect(),
            batch.operation_id.clone(),
            2,
            Vec::new(),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_job(index: u64) -> ReplicaReportJob {
        ReplicaReportJob::new(
            format!("block-{index}").into_bytes(),
            index + 1,
            format!("checksum-{index}").into_bytes(),
            format!("operation-{index}").into_bytes(),
        )
    }

    #[tokio::test]
    async fn batch_drains_only_ready_items_up_to_limit() {
        let (sender, mut receiver) = mpsc::channel(BATCH_MAX + 2);
        for index in 0..(BATCH_MAX + 2) {
            sender
                .send(report_job(index as u64))
                .await
                .expect("report queue accepts test job");
        }

        let first = receiver.recv().await.expect("first report");
        let batch = collect_batch(first, &mut receiver).await;

        assert_eq!(batch.jobs.len(), BATCH_MAX);
        assert_eq!(receiver.len(), 2, "overflow remains for the next batch");
        assert_eq!(
            batch.operation_id,
            ReplicaReportBatch::new(batch.jobs.clone()).operation_id
        );
    }
}
