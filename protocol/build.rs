fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/dms/v1/types.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/client_node.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/node_peer.proto");
    println!("cargo:rerun-if-changed=proto/dms/v1/node_meta.proto");
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(
            &[
                "proto/dms/v1/client_node.proto",
                "proto/dms/v1/node_peer.proto",
                "proto/dms/v1/node_meta.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
