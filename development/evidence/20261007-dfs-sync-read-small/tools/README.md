# DFS sync tools 20261007 r1

Current stable tool version is recorded in `summary.json`, `tool-provenance.json`, and `narrow-diff.patch`. This slice only changes `source/development/acceptance/dfs_manyread_small.py` and `source/development/acceptance/test_dfs_manyread_small.py`.

Stable protocol: `HELLO -> ACK0 -> READY -> START -> DONE -> ACKn -> ... -> FINAL`. ACK0 prevents a fast reader from sending READY1 before the peer has reached HELLO. ACKn prevents READY(n+1) or final full-SHA/EOF verification before the coordinator has received both DONE events and recorded `end_after_both_done_ns`. DONE requires `status=PASS`, `rc=0`, and a canonical C `seq-read close` result shape.

Validation/provenance:
- `raw/r4-linux-sync-guards.*`: Linux ARM64 root run on `afs-accept-a`, 6 sync-protocol affected guards, exit 0, OK. Current stable source SHA is bound in `summary.json` and `tool-provenance.json`; Linux `/var/tmp/afs-dfs-sync-tools-20261007-r3` now contains this final ACK0 version.
- `raw/r2-linux-guards.*`: retained only as historical base/default helper guard evidence. It is not bound to the current stable sync-protocol SHA and is not used as proof for ACK0/ACKn behavior.
- `raw/linux-guards.*`: first guard run failed because the expected missing-peer string was wrong; actual fail-closed behavior was `ValueError("expected HELLO, got 'READY'")`. Preserved as a failed guard version.
- `raw/r3-linux-sync-guards.*`: exit 0 but emitted ResourceWarning before pipe cleanup; exact Python sources are UNKNOWN after later overwrite.
- `raw/r3b-linux-sync-guards.*`: superseded INIT protocol; exact Python sources are UNKNOWN after later overwrite.
- `raw/provenance-linux-var-tmp.*`: readonly inventory of remaining Linux temp tool directories; no `.py` snapshots copied into this packet.

No real DFS mount/runtime workload was started here; parent/entry owns the runtime deployment and host relay.

Last evidence update: 2026-10-07T07:39:51+08:00.
