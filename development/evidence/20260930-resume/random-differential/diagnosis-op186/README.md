# v27 random op186 EBUSY diagnosis

Scope: read-only diagnosis plus minimal isolated repros on live v27 DFS. No product source edits, no Cargo/build, no service restarts, no fault injection.

## Original failure

- Original evidence: `../run-dfs-500-20260930T162214Z/`.
- Failure: op 186 `pwrite` to `d0/f0` at offset 1256 returned target `EBUSY`; ext4 reference wrote 20 bytes.
- Node log at the same replay class reports `DFS write lease changed or expired`; this maps to FUSE `EBUSY`.

## Repeat probes

- `diagnosis-repeat-1.json`: first mismatch at op 108, op `{'data_hex': '0bad1a0166ec30f83f7062c311d46df3770016ae5c3bb90ea913946e8e8eacfceecb31056263d5b4716b248cab02132d3ca4654bddf16243d4ccfdf271bf888397', 'index': 108, 'op': 'append', 'path': 'd3/f2'}`, target `{'errno': 16, 'errno_name': 'EBUSY', 'ok': False}`.
- `diagnosis-repeat-2.json`: first mismatch at op 161, op `{'data_hex': '929585303a099edf475747e0d51c034dc1ddd25cf60fe6e41934f2b5d3604d448800ebd6d419d4a6466f794aa72e5fe334c27b28a3', 'index': 161, 'op': 'write', 'path': 'f3'}`, target `{'errno': 16, 'errno_name': 'EBUSY', 'ok': False}`.

The exact operation index is timing dependent, but the failure mode repeats as target-only `EBUSY` during single-threaded write replay after enough time has elapsed for a DFS write lease to expire.

## Minimal timed repro

- Script: `min_expiry_repro.py`.
- Result: `{'initial_write': {'written': 6}, 'root': '/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v27/mount-dfs/diag-expiry-20260930T163455Z', 'second_pwrite': {'errno': 16, 'errno_name': 'EBUSY', 'ok': False}, 'sleep_seconds': 35, 'stat': {'size': 6}}`.

Interpretation: create/write/close a fresh DFS file, wait 35 seconds, then open+pwrite the same file. The second write returns `EBUSY`, confirming this is a lease-expiry/local-write-state bug rather than a path-specific `d0/f0` data bug.
