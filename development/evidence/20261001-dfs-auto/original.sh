#!/usr/bin/env bash
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
root=/home/lzc.guest/afs-build/work/dfs-auto-v75-original
out=/home/lzc.guest/afs-build/evidence/dfs-auto-v75-original
test ! -e "$root" && test ! -e "$out"
mkdir -p "$root" "$out"
tar -xf /tmp/afs-dfs-auto-v75-original.tar -C "$root"
cd "$root"
python3 - <<'PY'
import hashlib,json,pathlib
paths=sorted([pathlib.Path('Cargo.toml'),pathlib.Path('Cargo.lock'),*pathlib.Path('src').rglob('*.rs'),*pathlib.Path('crates').rglob('*.rs'),*pathlib.Path('crates').rglob('*.proto'),*pathlib.Path('crates').rglob('Cargo.toml')])
pathlib.Path('/home/lzc.guest/afs-build/evidence/dfs-auto-v75-original/before-graft-inputs.json').write_text(json.dumps({str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},indent=2)+'\n')
p=pathlib.Path('src/node/rpc/data.rs');s=p.read_text()
start=s.index('    #[cfg(all(feature = "dfs", feature = "rdma"))]\n    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]\n    #[ignore = "requires explicit Linux RXE device; product payload, not diagnostic NodeData"]\n    async fn dfs_product_rdma_replica_and_read_real_verbs()')
end=s.index('\n    #[cfg(feature = "dfs")]\n    struct AllowDfsTestGrant;',start)
graft=s[start:end].replace('dfs_product_rdma_replica_and_read_real_verbs','dfs_auto_prefers_rdma_replica_and_read_real_verbs').replace('DfsRdmaPool, PeerConnectionPool, RdmaChunkTransfer, RdmaReplicaDataPlane,','DfsRdmaPool, PeerConnectionPool, DataMode, make_replica_data_plane, make_chunk_transfer,').replace('DfsRdmaPool::new(peers, device,','DfsRdmaPool::new(peers.clone(), device,').replace('RdmaReplicaDataPlane::new(pool.clone()).unwrap()','make_replica_data_plane(DataMode::Auto, peers.clone(), std::time::Duration::from_secs(10), Some(pool.clone())).unwrap()').replace('RdmaChunkTransfer::new(pool).unwrap()','make_chunk_transfer(DataMode::Auto, peers, std::time::Duration::from_secs(10), Some(pool)).unwrap()').replace('DFS_PRODUCT_RDMA','DFS_AUTO_RDMA')
p.write_text(s[:end]+'\n'+graft+s[end:])
pathlib.Path('/home/lzc.guest/afs-build/evidence/dfs-auto-v75-original/graft-test.txt').write_text(graft+'\n')
pathlib.Path('/home/lzc.guest/afs-build/evidence/dfs-auto-v75-original/grafted-inputs.json').write_text(json.dumps({str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},indent=2)+'\n')
PY
rdma link show > "$out/environment.txt"
printf '%s\n' 'env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_prefers_rdma_replica_and_read_real_verbs -- --ignored --nocapture' > "$out/original.command"
set +e
env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_prefers_rdma_replica_and_read_real_verbs -- --ignored --nocapture > "$out/original.log" 2>&1
result=$?
set -e
printf '%s\n' "$result" > "$out/original.exit"
tail -n 24 "$out/original.log"
exit "$result"
