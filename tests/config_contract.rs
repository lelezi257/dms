use afs::config::{Cli, Config, MetaStoreBackend, Role};
use clap::Parser;
#[test]
fn file_values_are_overridden_only_by_explicit_cli() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node.toml");
    std::fs::write(&path, "id='file-node'\ngrpc_listen='127.0.0.1:9901'\n").unwrap();
    let cli = Cli::parse_from([
        "afs-node",
        "--config",
        path.to_str().unwrap(),
        "--id",
        "cli-node",
    ]);
    let cfg = Config::resolve(Role::Node, cli).unwrap();
    assert_eq!(cfg.id, "cli-node");
    assert_eq!(cfg.grpc_listen.to_string(), "127.0.0.1:9901");
}
#[test]
fn unknown_field_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "fs_mdoe='all'").unwrap();
    let cli = Cli::parse_from(["afs-node", "--config", path.to_str().unwrap()]);
    assert!(
        Config::resolve(Role::Node, cli)
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );
}
#[test]
fn invalid_limits_and_uncompiled_mode_fail() {
    let cli = Cli::parse_from(["afs-node", "--timeout-ms", "0"]);
    assert!(Config::resolve(Role::Node, cli).is_err());
    #[cfg(not(feature = "ownerfs"))]
    {
        let cli = Cli::parse_from(["afs-node", "--fs", "ownerfs"]);
        assert!(Config::resolve(Role::Node, cli).is_err());
    }
    #[cfg(not(feature = "blobfs"))]
    {
        let cli = Cli::parse_from(["afs-node", "--fs", "blobfs"]);
        assert!(Config::resolve(Role::Node, cli).is_err());
    }
}

#[test]
fn meta_store_defaults_to_etcd_and_cli_overrides_toml() {
    let default = Config::resolve(Role::Meta, Cli::parse_from(["afs-meta"])).unwrap();
    assert_eq!(default.meta_store, MetaStoreBackend::Etcd);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.toml");
    std::fs::write(&path, "meta_store = 'memory'\n").unwrap();
    let file = Config::resolve(
        Role::Meta,
        Cli::parse_from(["afs-meta", "--config", path.to_str().unwrap()]),
    )
    .unwrap();
    assert_eq!(file.meta_store, MetaStoreBackend::InMemory);

    let cli = Config::resolve(
        Role::Meta,
        Cli::parse_from([
            "afs-meta",
            "--config",
            path.to_str().unwrap(),
            "--meta-store",
            "etcd",
        ]),
    )
    .unwrap();
    assert_eq!(cli.meta_store, MetaStoreBackend::Etcd);

    let local = Config::resolve(
        Role::Meta,
        Cli::parse_from(["afs-meta", "--meta-store", "local-file"]),
    )
    .unwrap();
    assert_eq!(local.meta_store, MetaStoreBackend::LocalFile);
}
