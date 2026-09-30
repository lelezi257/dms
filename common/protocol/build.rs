// 正式 wire 协议唯一生成入口。Tonic 生成 client、server trait 和 service 路由，
// Prost 生成消息类型；Node/Meta 只实现对应 trait，无需手写 gRPC 方法注册表。
// RDMA 只替换内容搬运路径，因此 node_data.proto 的文件命令/结果仍需生成。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = [
        "proto/error.proto",
        "proto/local_api.proto",
        "proto/meta.proto",
        "proto/node_data.proto",
    ];
    for file in &files {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-changed=proto/node_control.proto");
    tonic_prost_build::configure().compile_protos(&files, &["proto"])?;
    // The public Rust modules are flat; map imported data authority types to
    // their existing module while generating the control service separately.
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let control_out = out_dir.join("control_codegen");
    std::fs::create_dir_all(&control_out)?;
    tonic_prost_build::configure()
        .out_dir(&control_out)
        .extern_path(".afs.node.data.v1", "crate::node_data")
        .compile_protos(&["proto/node_control.proto"], &["proto"])?;
    // Tonic also emits imported service shells; copy only control output so
    // those shells cannot overwrite the first pass's complete data schema.
    std::fs::copy(
        control_out.join("afs.node.control.v1.rs"),
        out_dir.join("afs.node.control.v1.rs"),
    )?;
    Ok(())
}
