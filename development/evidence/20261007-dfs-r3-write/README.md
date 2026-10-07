# Current DFS R3 small writes

2026-10-07, independent G2.23 observation. Product7e6e00a6/map151a2c6d,
ordinary packagec3bb5a30; no Rust/vendor change or rebuild. [Frozen plan](../../dfs-r3-write-small.md), [goal table](../../trial-release-goals.md).

One A writer, six new64MiB files,1MiB blocks,C1,local-file/gRPC/defaultOFF.
Round00 warmup;round01..05 measured. Counter generations1..6 give96 distinct
4MiB chunks across files, all retained; no cross-round content deduplication.
C timer open/write/fdatasync/close excludes Python SHA/EOF and directory fsync.
Values78.456951/86.531972/81.930440/90.821795/78.768173MiB/s,
median81.930440MiB/s. No qualified3FS parity, cache/RPC placement unobserved.

Each round: full writer SHA/EOF+directory fsync, independent exact directory
entry/inode/head/version/layout selection,16 chunks×3 Ready/Durable CopyRecords
and48 physical byte/SHA checks on A/B/C before next round;B/C new-open full
SHA/EOF/read probe also passed. This is derived durable CopyRecord evidence,
not original complete ReplicaAck or power-loss durability. Three VMs share
one physical host; no claim of three physical failure domains.

Four services actualwait0, eight bound child/supervisor PIDs gone, three FUSE
mounts/UDS normally closed,11 protected process identities and complete initial
mount inventories preserved. Peak final accounted1,615,421,440B < predeclared
new-case2GiB aggregate. Existing per-role1GiB and backing free1GiB retained;
this new budget does not change earlier failed budgets. Data copies need1152MiB,
package/tools roughly388MB; full23,261B logs are not the capacity driver.
The original fixture's historical parent-scope text says1GiB; this new case
explicitly declares2GiB in the plan/admission before service start.

Linux8 C qualification+7 driver guards+6 independent observer guards passed.
Initial driver test failed only on stale expected error text; original15-test
output retained then corrected guard passed. Host preparation first expected
an incorrect patch status; actual patch was saved/reused unchanged. Exactly
one product runtime attempt; no environment repairs or score-based reruns.

[Independent Linux record verification](verify-report.json) passed the exact
identity, raw timer, content, replica, budget and closure checks. The verifier's
four initial harness/input failures are retained in [attempts](verifier-attempts.json)
and the original command/archive index; correcting those did not rerun services.
The original candidate snapshot contains two inherited, unused helper hashes;
[metadata audit](candidate-metadata-audit.json) explicitly distinguishes these
from the current tool/observer input maps. The echoed candidate fields in the
verifier report are not a claim that those old helpers were used.

Full logs retained,39ERRO and60WARN preserved:33 dentryNotFound,6 non-userxattr,
60 terminalioctl21505. Their auxiliary-probe origin is inference without request
IDs, not a zero-error or fullPOSIX claim. Timed/content/replica checks passed
independently. Evidence indices bind compact result files to original raw
commands, observer sources, archives and data identities outside Git; no
script snapshots, payloads, ELF or private TLS copied into this packet.

G1 historical8/8 stays closed; G2.23 formal3FS comparison, G2.27 and fullbind
remain open. OrdinaryOwner1.2×MooseFS throughput/.8×latency criteria stay in
force and this DFS observation does not supply its missing latency evidence.
Next independent item: G2.24 small multi-node read/write; big/long/complex
reliability/etcd/Redis and3FS baseline qualification remain deferred.
