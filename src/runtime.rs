//! Process-owned observability and bounded service lifecycle. No business authority here.
//!
//! 三种公共设施分工：logging 输出事件；metrics 聚合次数；tracing 关联一次跨服务请求。
//! 每个进程只初始化一次全局日志/Trace，SDK 复用调用进程的上下文，不自行安装 subscriber。
//! Services 管 Tokio 长运行任务；FUSE BackgroundSession 与本机 UDS 的资源归 Node 管。
//! 不创建全局 actor 队列：不同 RPC 可并发，只有具体业务需要的状态才局部加锁。

use crate::config::Config;
use afs_metrics::{IntCounterVec, Opts, Registry};
use std::{future::Future, time::Duration};
use tokio::{sync::watch, task::JoinSet};
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type ServiceResult = Result<(), BoxError>;
#[derive(Clone)]
/// 每进程独立的指标注册表和基础请求计数；不把文件路径等高基数字段当指标标签。
pub struct Observability {
    pub registry: Registry,
    requests: IntCounterVec,
}
impl Observability {
    pub fn new() -> Result<Self, BoxError> {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("afs_requests_total", "Completed foundation operations"),
            &["component", "operation", "result"],
        )?;
        registry.register(Box::new(requests.clone()))?;
        Ok(Self { registry, requests })
    }
    /// All callers use bounded static labels; never put paths or request IDs in labels.
    pub fn record(&self, component: &'static str, operation: &'static str, ok: bool) {
        self.requests
            .with_label_values(&[component, operation, if ok { "ok" } else { "error" }])
            .inc();
    }
}
/// 进程退出前保留的日志/Trace 运行句柄；Drop 负责对应观测设施的收尾。
pub struct ProcessGuards {
    _logging: afs_logging::LoggingGuard,
    _tracing: afs_tracing::TracingGuard,
}
pub fn initialize(
    cfg: &Config,
    service: &str,
    obs: &Observability,
) -> Result<ProcessGuards, BoxError> {
    let _logging = afs_logging::init_process_logging(
        &afs_logging::LoggingConfig {
            level: afs_logging::parse_level(&cfg.log_level)?,
            ..Default::default()
        },
        afs_logging::ProcessIdentity::new(service, &cfg.id),
    )?;
    let _tracing = afs_tracing::init_process_tracing(
        &afs_tracing::TracingConfig {
            enabled: cfg.trace_enabled,
            otlp_endpoint: cfg.trace_endpoint.clone(),
            sample_ratio: cfg.trace_sample_ratio,
            ..Default::default()
        },
        afs_tracing::ProcessIdentity::new(service, &cfg.id),
        Some(afs_metrics::TraceRuntimeMetrics::register(&obs.registry)?),
    )?;
    Ok(ProcessGuards { _logging, _tracing })
}
pub async fn cancelled(mut rx: watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            break;
        }
    }
}
/// 一组同生共死的异步服务。任一服务意外结束应让进程退出，避免只剩 health 正常。
pub struct Services {
    pub stop: watch::Sender<bool>,
    tasks: JoinSet<ServiceResult>,
}
impl Default for Services {
    fn default() -> Self {
        Self::new()
    }
}
impl Services {
    pub fn new() -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            stop,
            tasks: JoinSet::new(),
        }
    }
    pub fn spawn(&mut self, future: impl Future<Output = ServiceResult> + Send + 'static) {
        self.tasks.spawn(future);
    }
    /// 等待 SIGINT/SIGTERM 或服务退出，然后广播停止并限时排空。
    /// 超时会中止异步任务并返回错误；不把强制退出报告成正常关闭。
    pub async fn run(mut self) -> ServiceResult {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result = tokio::select! {
            result=tokio::signal::ctrl_c()=>result.map_err(Into::into),
            _=terminate.recv()=>Ok(()),
            task=self.tasks.join_next()=>match task {
                Some(Ok(Err(e)))=>Err(e),Some(Err(e))=>Err(e.into()),
                _=>Err(std::io::Error::other("service exited unexpectedly").into()),
            }
        };
        let _ = self.stop.send(true);
        let mut failure = None;
        let drained = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(task) = self.tasks.join_next().await {
                match task {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failure = Some(error),
                    Err(error) => failure = Some(error.into()),
                }
            }
        })
        .await;
        if drained.is_err() {
            self.tasks.abort_all();
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                while self.tasks.join_next().await.is_some() {}
            })
            .await;
            afs_logging::error!("process.shutdown_forced");
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "service shutdown exceeded deadline; tasks aborted",
            )
            .into());
        }
        if let Some(error) = failure {
            return Err(error);
        }
        afs_logging::info!("process.stopped");
        result
    }
}
