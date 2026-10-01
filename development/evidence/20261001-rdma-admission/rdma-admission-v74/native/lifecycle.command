env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --test rdma_lifecycle -- --ignored --test-threads=1 --nocapture 
