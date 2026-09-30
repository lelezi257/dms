> Checkpoint subset: failure-r2/, original fixture scripts and root commands.log remain in the original research workspace. This copy preserves repaired/ proof and result.json; it is historical, not the current v25 runtime.

# A/B real product RDMA probe

Development evidence only; release acceptance is NOT RUN. Memory-backed Meta does not prove metadata persistence.

## Original candidate: FAIL

Node SHA256 `e6f50d8747ec63ffc2a88f33be64bafbdb14db4785c92d05899971b750d68a10`; Meta `0d3598ff32f2924770ca4b52350e55eaca663e05b0a99771bf2386a7eb47f942`. Source manifest aggregate `5122da677b68e402dc39b1e657975f1779f416e6241ba91d9361617010db8512`; excludes subsequent OwnerFs alias repairs.

Real A/B ARM64 Linux, ext4, mTLS, forced `data_mode=rdma`, memory Meta R2/M2 with local-required and two distinct nodes/failure domains. A wrote 4MiB+17; `fsync` returned EHOSTUNREACH. A contains staged durable chunks; B contains none; no successful file commit is claimed. Raw results are preserved under `failure-r2/`.

Facts: native QP creation hard-coded GID table index0, A/B index0 is IPv6 link-local, IPv6 peer ping failed, IPv4 peer ping succeeded, index1 is IPv4-mapped. QP RTR transition failed with errno110. The missing B verbs device was prepared using `modprobe ib_uverbs`, `modprobe rdma_rxe`, `rdma link add rxe0 type rxe netdev eth0`; no existing service was restarted. Initial script permission/quoting preparation failures remain in commands.log.

Isolated ports18420/18421 +18430/18431 were stopped and mounts unmounted. Existing178xx services and ctl175xx were left untouched. Guest data free19GiB/30GiB exceeds4GiB floor.

## Native repair

Only `common/transport/native/rdma.c` changes in the isolated428file snapshot. Verbs50.0 installed headers/man confirm `ibv_query_port.gid_tbl_len`, indexed `ibv_query_gid` and uint8 `sgid_index`. Choose a nonzero IPv4-mapped GID preferentially, valid global IPv6 then link-local otherwise. Store chosen index, advertise its GID, use the same index when connecting. Reject incompatible peer family with a precise error. Log selected index/address/family; do not add gRPC fallback.

Candidate source aggregate `fcf67a71e62b720fe9820f5950a95ac047aaa3ea0a80450c24071acf24bb83a5`. Linux gnu11 warnings-as-errors syntax check passed. Build and repaired product fixture passed as described below.

## Repaired candidate: development probes PASS

Node SHA256 `7c14e03f63041440d74ad003d79ec04f5503fbc225cbe3dbf16b88997e8ce42e`; Meta `507c44337efb432d6762302deb69313a635831b5fd4d328f1a910c87bffe09fa`. These binaries are the original identified p3 snapshot plus the single GID repair; they do not include later OwnerFs alias fixes or later backend work. Binaries are preserved on the host under `.local/p3-rdma-cross/p3-rdma-gid-binaries/`. Guest duplicate binary archives were removed only after host copy/hash verification. Build VM disk pressure was reported to root; recoverable caches/duplicate archives restored capacity; no new Cargo was started afterward.

[Structured result](result.json), [source manifest](gid-source.sha256), [Linux build log](build-gid.log), [primary verbs declarations/manual](verbs-primary.txt).

### R2/M2: synchronous replica persistence

Independent memory Meta on A, new A/B data directories, ports18420/18421 and18430/18431, forced RDMA and mTLS. A FUSE wrote/fsynced/closed4194321 bytes successfully. B received exactly4194321 RDMA replica bytes, with zero gRPC file payload. B's4MiB and17-byte chunks have the expected content checksums, both Durable catalog records. After stopping B, the catalog and chunk files remained complete; after restarting the same Node from the same ext4 data directory, its local recovery passed and the catalog/chunk contents were unchanged. Meta stayed running for this local recovery check; this does not prove Meta persistence or total cluster recovery. R2 does not claim a remote read path.

Raw evidence: `repaired/r2/a-write.json`, `b-after.metrics`, `b-catalog-after-stop.json`, `b-catalog-recovered.json`, `b-node-transfer.log`, `b-node-recovery.log`. Native selected `::ffff:192.168.109.12`/`.13`, index1 discovered dynamically. B logged real RDMA READ completions for4194304 and17 bytes, then returned durable replica ACKs.

### R1/M1: actual FUSE peer read

A separately initialized memory Meta with new R1 directories and ports18440/18441 +18450/18451. A FUSE wrote/fsynced/closed the same4194321-byte file. B's local Chunk store was empty before reading and remained empty afterward. B read the complete file through its actual FUSE mount, matching every byte and SHA256 `7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231`. A's real RDMA read-response payload counter increased4194321 bytes; gRPC payload counters remained zero. A's native RDMA WRITE completions show32*128KiB+17 bytes. This is distinct from a B local-copy read in the R2 fixture.

Raw evidence: `repaired/r1/a-write.json`, `b-catalog-before-read.json`, `b-read.json`, `b-catalog.json`, `a-after.metrics`, `a-node.log`. The configured4MiB transfer/chunk boundary did not cap the file at4MiB.

### Authentication, cleanup and limits

Both product fixtures use HTTPS peer/Meta endpoints, the existing acceptance CA, per-node identity certificates and exact trusted-node certificate mappings. Successful product calls use the authenticated grant/peer paths. The captured `openssl s_client` EOF probe shows the certificate request and server certificate but does **not** establish negative no-client-certificate rejection; it is not counted as such. Adversarial authorization regressions remain a separate earlier P3b evidence lane.

All new product processes and FUSE mounts were stopped/unmounted; guest data and raw logs retained. No175xx or178xx service/mount was restarted by these fixtures. Data disks remain above4GiB free. The original failed candidate and initial fixture scripting failures are preserved rather than relabeled as passing. No gRPC fallback was enabled. This small memory-backed development evidence does not qualify release, backend persistence, long soak, repair, full RDMA security matrix or comparative performance.

## Reproduction

`fixture.py` creates/configures only the named fresh A/B fixture directories. `repaired-fixture.py r2 2` and `repaired-fixture.py r1 1` bind the repaired candidate run. Change RUN to a fresh suffix before repeating; existing data directories are intentionally rejected. Stage the identified Node/Meta binaries in `/var/tmp/afs-rdmax-bin/` and prepare real `rxe0`/verbs access first. Linux uses root for FUSE/memlock only. `probe.py` performs real POSIX write/fsync/close/read; `summarize.py` checks raw receipts/catalogs/payload counters. `commands.log` and `repaired/commands.log` contain exact commands/output/exit status. Resume scripts preserve the visible intermediate collection failure (ps after deliberately stopping B), then complete recovery and cleanup.

## Integration change rationale

Suggested execution change record: native endpoint GID index must be discovered from the selected active port, never assumed0. The advertised GID and QP source index must match. IPv4-mapped preference supports the agreed A/B IPv4 RXE environment; valid IPv6 alternatives remain available with family mismatch diagnosed. This does not change the RPC protocol, replica completion contract or frozen Meta commit request. Multi-homed route-aware GID configuration and physical hardware performance remain unqualified by this fixture.
