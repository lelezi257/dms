env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_prefers_rdma_replica_and_read_real_verbs -- --ignored --nocapture
