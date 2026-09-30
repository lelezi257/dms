# processctl v50 runtime product probe

Status: PASS.

This is a bounded Linux A product lifecycle proof for current `afs-processctl`
with v49-qualified binaries. It is not a full DEP/REL acceptance gate.

## Identity

- VM: `afs-accept-a`
- Runtime path: `/mnt/lima-afsadata/afs-delivery/processctl-v50`
- Filesystem: ext4 `/dev/vdb1` mounted at `/mnt/lima-afsadata`
- Ports: `18080..18083`
- Final run id: `20260930T231148Z-pid715695`
- Controller SHA256: `b5dd0fa011475fb94658d3b1322ed8395c69aa1f85fbff5b5e1e4a307eec83a1`
- Node SHA256: `dbbf2ccd5eb47f06178bfc3140599865c1b7ad5f28851e244e728fbcdfc136af`
- Meta SHA256: `917432057950380da208b057a4d94156841e1eab03e533f56675433d5184fd9c`
- Config SHA256: Node `deec90fa4c6cc34fae309511ed72dc1f6a3990be54e108efebae4998196244b6`, Meta `f72ddc3554aae64cd5a96e8f04ae09e27a467590fe32de7cc566bb3c7d649c0e`

## Result

- `processctl start all` returned 0 and both services reached readiness with exact FUSE mounts: `afs-dfs` at `mount-dfs`, `afs-ownerfs` at `mount-ownerfs`.
- Ordinary held writable FD case: accepted dirty bytes were written, `processctl stop node` returned 0, controller status reported `stopped exit_code=0`, lifecycle receipt reported `exit_code=0`, and first restart read back `accepted-dirty`.
- Paused Meta case: Meta PID `715814` stayed the same incarnation and was observed in state `T`; the dirty write was issued after that `T` proof and accepted 4 bytes. `processctl stop node` returned 124 after 20.12368465399777 seconds, controller status reported `failed exit_code=124`, and lifecycle receipt reported `exit_code=124`.
- Because this run used the 124 timeout path, the timeout evidence is the 20.12368465399777 second measured controller stop duration; the raw log also contains `process.shutdown_forced`. The exit-1 log-marker branch was not used in the final run. The native 15-second watchdog starts when service shutdown begins; this probe does not establish a 15-second signal-to-exit bound. Whole-shutdown budget qualification remains open.
- After `SIGCONT` on Meta and explicit `processctl start node`, the first fresh read of the forced file returned the acknowledged seed watermark `seed`; the prior normal file still read `accepted-dirty`.
- Final `processctl stop node` and `processctl stop meta` both returned 0 with matching status and lifecycle receipts.
- Final independent check found no listeners on ports `18080..18083` and no FUSE mounts left at the fresh runtime path.

## Evidence

- Raw report: `guest/report.json`
- Controller output: `runtime.stdout`, `runtime.stderr`
- Configs: `guest/etc__meta.toml`, `guest/etc__node.toml`
- SHA manifests: `manifest.json`, `guest/staged-binaries.sha256`, `guest/controller.sha256`, `guest/config.sha256`
- Logs: `guest/log__meta.log`, `guest/log__node.log`
- Final identity files: `guest/run__meta.identity`, `guest/run__node.identity`
- Raw lifecycle snapshots for normal stop, failed stop and final stops: `guest/probe-artifacts-20260930T231148Z-pid715695/`
- Earlier strengthened pass archived before final raw-lifecycle rerun: `attempts/20260930T231043Z-post-t-dirty-124-pass-archived/`
- Earlier prepause-order pass archived before strengthening: `attempts/20260930T231007Z-prepause-pass-archived/`
- Archived harness-only failure before the sudo runner fix: `attempts/20260930T230357Z-permission-probe-fail/`

No TLS private key contents were copied into evidence; only public certificate metadata is recorded in `guest/tls-node-a-public.txt`.
