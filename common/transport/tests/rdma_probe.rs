#![cfg(feature = "rdma")]

use afs_transport::rdma::RdmaEndpoint;

fn rdma_device() -> Option<String> {
    std::env::var("AFS_TEST_RDMA_DEVICE")
        .ok()
        .filter(|value| !value.is_empty())
}

fn connected_pair(device: &str) -> (RdmaEndpoint, RdmaEndpoint) {
    let mut client = RdmaEndpoint::open(device).expect("client endpoint");
    let mut server = RdmaEndpoint::open(device).expect("server endpoint");
    let client_info = client.info().expect("client descriptor");
    let server_info = server.info().expect("server descriptor");

    // 真实握手顺序和 Node control 一致：server 先准备 RECV，再向 client 公开
    // server_info。这样 client 的零字节 SEND_WITH_IMM 到达时一定有接收槽。
    server.prepare_probe().expect("server posts probe receive");
    server
        .connect(&client_info)
        .expect("server connects client");
    client
        .connect(&server_info)
        .expect("client connects server");
    (client, server)
}

#[test]
#[ignore = "requires AFS_TEST_RDMA_DEVICE with a working RXE/RDMA device"]
fn rdma_probe_success_keeps_one_sided_transfers_usable() {
    let device = rdma_device().expect("explicit RXE tests require AFS_TEST_RDMA_DEVICE");
    let (mut client, mut server) = connected_pair(&device);

    client.send_probe(5000).expect("client probe send");
    server.wait_probe(5000).expect("server probe receive");

    client.put_local(b"abcdefgh").expect("client puts bytes");
    server.transfer_read(8).expect("server reads client MR");
    assert_eq!(
        server.get_local(8).expect("server local bytes"),
        b"abcdefgh"
    );

    server.put_local(b"ABCDEFGH").expect("server puts bytes");
    server.transfer_write(8).expect("server writes client MR");
    assert_eq!(
        client.get_local(8).expect("client local bytes"),
        b"ABCDEFGH"
    );
}

#[test]
#[ignore = "requires AFS_TEST_RDMA_DEVICE with a working RXE/RDMA device"]
fn rdma_probe_wait_without_client_send_times_out_and_poisons_endpoint() {
    let device = rdma_device().expect("explicit RXE tests require AFS_TEST_RDMA_DEVICE");
    let (_client, mut server) = connected_pair(&device);

    let error = server
        .wait_probe(25)
        .expect_err("missing client probe must time out");
    assert!(error.to_string().contains("probe receive timeout"));
    assert!(
        server
            .transfer_write(1)
            .expect_err("timed-out probe poisons endpoint")
            .to_string()
            .contains("poisoned")
    );
}

#[test]
#[ignore = "requires AFS_TEST_RDMA_DEVICE with a working RXE/RDMA device"]
fn rdma_probe_rejects_duplicate_send() {
    let device = rdma_device().expect("explicit RXE tests require AFS_TEST_RDMA_DEVICE");
    let (mut client, _server) = connected_pair(&device);

    client.send_probe(5000).expect("first probe send");
    let error = client
        .send_probe(5000)
        .expect_err("second probe send must be rejected");
    assert!(error.to_string().contains("probe already sent"));
}
