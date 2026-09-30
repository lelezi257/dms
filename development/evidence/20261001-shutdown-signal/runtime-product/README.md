# shutdown signal v51 runtime product probe

Status: PASS.

This is a bounded Linux A direct-SIGTERM product lifecycle proof for current
`afs-processctl` with v51-qualified binaries. It is not a full DEP/REL
acceptance gate.

## Identity

- VM: `afs-accept-a`
- Runtime path: `/mnt/lima-afsadata/afs-delivery/shutdown-signal-v51`
- Filesystem: ext4 `/dev/vdb1` mounted at `/mnt/lima-afsadata`
- Ports: `18180..18183`
- Run id: `20260930T233048Z-pid719442`
- Controller SHA256: `b5dd0fa011475fb94658d3b1322ed8395c69aa1f85fbff5b5e1e4a307eec83a1`
- Node SHA256: `97d735243d741ef98d917f05a965c9e02703b3b05468ba42e7b1d7c5f2c38e31`
- Meta SHA256: `00cb99fe46f20024ab3b476978dd0e16d9e8a2c0535b95d5c8febc3aa863cf2d`

## Result

- `processctl start all` returned 0 and both services reached readiness with exact FUSE mounts.
- Ordinary held writable FD case: accepted dirty bytes were written, `processctl stop node` returned 0, status and lifecycle receipt both reported `exit_code=0`, and first restart read back `accepted-dirty`.
- Paused Meta case: Meta PID `719500` stayed the same incarnation and was observed in state `T`; the dirty write was issued after that `T` proof and accepted 4 bytes in `5.003018741001142` seconds.
- Direct Node signal case: the exact Node PID/start-ticks/hash were verified immediately before `SIGTERM`; signal-to-receipt elapsed time was `5.034410228996421` seconds.
- The controller then observed the existing receipt via `processctl stop node`: return code 1, status `failed exit_code=1`, lifecycle receipt `exit_code=1`.
- The exit-1 branch markers were present in the post-signal log: `dfs.node_drain_incomplete` and `node.shutdown_failed`.
- After `SIGCONT` on the same Meta and explicit `processctl start node`, the first fresh read of the forced file returned acknowledged seed watermark `seed`; the prior normal file still read `accepted-dirty`.
- Final `processctl stop node` and `processctl stop meta` both returned 0 with matching status and lifecycle receipts.
- Final independent check found no listeners on ports `18180..18183` and no FUSE mounts left at the v51 runtime path.

## Evidence

- Raw report: `guest/report.json`
- Explicit direct guest report copy: `guest-explicit-20260930T233254Z/report.json`
- Guest report SHA256: `c156f904d126646d5217569eaf4d592a0b31babbadd3ba6e9ca295b1931793f3` on both host copy and A-side source
- Controller output: `runtime.stdout`, `runtime.stderr`
- Configs: `guest/etc__meta.toml`, `guest/etc__node.toml`
- SHA manifests: `manifest.json`, `guest/staged-binaries.sha256`, `guest/controller.sha256`, `guest/config.sha256`, `guest-explicit-20260930T233254Z/`
- Logs: `guest/log__meta.log`, `guest/log__node.log`
- Final identity files: `guest/run__meta.identity`, `guest/run__node.identity`
- Raw lifecycle snapshots for normal stop, direct-signal failed stop and final stops: `guest/probe-artifacts-20260930T233048Z-pid719442/`

No TLS private key contents were copied into evidence; only public certificate metadata is recorded in `guest/tls-node-a-public.txt`.
