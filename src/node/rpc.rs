//! Node 内部通信边界。
//!
//! control/data 接收其他 Node 请求；peer 调用其他 Node；meta 调用中心。
//! 控制统一 gRPC，文件内容可用 gRPC 或单边 RDMA。通道选择不改变业务成功合同。
//! 本模块理解文件操作，公共 transport 只处理传输机制；业务与资源锁归业务所有者。

pub mod control;
pub mod data;
pub mod meta;
pub mod peer;
