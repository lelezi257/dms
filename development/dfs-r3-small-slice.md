# Current DFS three-copy small slice

2026-10-07, G2.21 bounded subitem. Main is the development/publication entry.

Before edits, the existing synchronized helper and receipt checks were inspected:
they retain the historical 6d/R2/uniform-content identity. Reuse their unchanged
stdio control protocol, not their payload or candidate constants. Add one narrow
first-party counter-pattern C probe and a current-input driver; no Rust, vendor,
protocol or replication architecture change is required. Lock corruption, EOF,
dataset/identity and three-copy policy checks with Linux tests before runtime.

The frozen product is 93169c8530beedff9e8e510dc9b9210d704f244f, compiler map
9661a313fb8b433caa00d03df5be43f880ad97859fc0299cd6d397cdf55cda0f. Reuse the
verified c7ea49d7 package and 4150942f Meta/9478f3e8 Node ELFs. Main c1bdb1cc
adds documentation/tools only relative to those compiler inputs. Installation
and all binaries/tools/TLS/configs must be admitted before any service starts.

Fresh roots are `/mnt/lima-afs{ctlstate,adata,bdata,cdata}/afs-delivery/dfs-r3-current-20261007-r1`
on the existing ctl/A/B/C Linux/ext4 VMs, Meta ports24700/24701 and Node24800/24801.
Use DFS-only, local-file Meta, gRPC, both workspace switches OFF, and
desired/sync-required/min-distinct-nodes/min-distinct-failure-domains all3 with
local-copy required. These are three VM/Node identities on one physical host;
they do not establish three physical failure domains. Policy is initialized in
fresh Meta state, never changed over historical R2 state.

A writes one64MiB file using64 counter-prefixed1MiB blocks (`counter-1m-v1`),
fdatasync/close and directory fsync. Require sixteen different4MiB chunk contents,
receipt-derived ReadyDurable metadata on three distinct nodes and the independent
physical bytes/SHA on all three stores before readers run. Persisted CopyRecords
do not expose the complete original ReplicaAck; do not claim otherwise.
B/C each perform full SHA/EOF precheck, one unmeasured warmup, five coordinated
fresh-open reads and full SHA/EOF postcheck. Reuse the ctl Linux monotonic
START-to-both-DONE window and retain each reader's C timing separately; host relay
and launch/result overhead are included in the common window. Cache residency
and actual RPC read path remain unobserved; no cold-cache or remote-read claim.

One bounded attempt, 60-second per-operation/control timeout, 30-second service
start/stop timeout. Retain data, failures and exact identities; stop an affected
lane on real environment blockage and ask for help. No environment repair or
unchanged rerun. Admission requires1GiB free reserve above a four-role aggregate
1GiB allocation ceiling; check each backing ext4 volume, never statfs the R3 FUSE
mount. Budget walks exclude mounted FUSE subtrees. Preserve every pre-existing
captured process and mount. Require all four supervised services' actual wait0,
all owned child/supervisor identities gone, exact mounts normally removed, and
protected identities unchanged. Remove no historical state.

This closes only current three-copy content/timing evidence if successful.
Matched, qualified3FS performance parity remains pending; its qualification
failure stays in the user-deferred topic. G1 historical8/8 and G2 major counts
do not change. Publish compact raw outputs/commands/identities and indices on
main; retain full logs/binaries/TLS outside Git with hashes/recovery paths.

Outcome: the declared frozen931 content/copy/lifecycle slice is complete;
[recorded data and limitations](evidence/20261007-dfs-r3-small/README.md).
Current7e6 regression and qualified3FS parity remain separate open items.

The separate [current7e6 normal recovery result](evidence/20261007-dfs-r3-current-recovery/README.md) now closes that bounded compatibility/recovery subitem. It does not relabel the931 five-round timing or close qualified3FS parity.
