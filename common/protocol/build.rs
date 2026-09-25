// 正式 wire 协议唯一生成入口。Tonic 生成 client、server trait 和 service 路由，
// Prost 生成消息类型；Node/Meta 只实现对应 trait，无需手写 gRPC 方法注册表。
// RDMA 只替换内容搬运路径，因此 node_data.proto 的命令/结果仍然需要生成。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = [
        "proto/local_api.proto",
        "proto/meta.proto",
        "proto/node_control.proto",
        "proto/node_data.proto",
    ];
    for file in &files {
        println!("cargo:rerun-if-changed={file}");
    }
    tonic_prost_build::configure().compile_protos(&files, &["proto"])?;
    Ok(())
}
