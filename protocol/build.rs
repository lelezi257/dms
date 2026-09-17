fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/dms/v1/types.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/client_node.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/node_peer.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/node_meta.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/filesystem_meta.proto");
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        // Peer payload 使用 `Bytes`，让 Tonic/Prost 可以借用解码帧；接收 Node
        // 随后只需复制一次进入 Arena，不先落到临时 Vec 再复制第二遍。
        // 只改 Node 内部协议字段，不改变公开 SDK DTO。
        .bytes(".dms.v1.PeerPullBlockResponse.payload")
        .bytes(".dms.v1.PeerPullBlockChunk.payload")
        .compile_protos(
            &[
                "proto/dms/v1/client_node.proto",
                "proto/dms/v1/node_peer.proto",
                "proto/dms/v1/node_meta.proto",
                "proto/dms/v1/filesystem_meta.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
