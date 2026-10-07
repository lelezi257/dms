# DFS R3 small one-writer/two-reader result

2026-10-07. **Fact: bounded content/copy/lifecycle PASS; timing data recorded.**
Product source **93169c8530beedff9e8e510dc9b9210d704f244f**, compiler map
9661a313fb8b433caa00d03df5be43f880ad97859fc0299cd6d397cdf55cda0f;
Meta4150942f / Node9478f3e8 / packagec7ea49d7 remain the previously frozen
[inputs](../20261007-dfs-r3-preparation/package-reuse.json). Latest main7e6e00a6
has different compiler inputs; this run does **not** establish its DFS regression.
G1 historical8/8 remains closed; G2.21 qualified3FS parity and major counts stay open.

Four-role Linux preflight, probe dependencies and exact protected inventory passed
before normal Meta/three-Node startup. Three configured VM/node domain identities
are on one physical host, not three physical failure domains. Bind switches OFF.

A wrote64MiB counter-prefixed1MiB blocks, fdatasync/close and directory fsync;
full content SHA4c47e859a6831026ba9367262afb1e1803d11abc30f6f3d996ceea829af89324
and EOF passed. Before readers started, [independent onsite proof](replica-proof.json)
verified16 different4MiB chunks, three distinct Ready/Durable CopyRecords each,
matching durable catalogs and all48 original physical chunk byte patterns/SHA.
No192MiB chunk archive was made. CopyRecords are derived receipt facts, not
complete persisted original ReplicaAck objects.

B/C each passed complete pre/post SHA and EOF, one unmeasured warmup, five
coordinated fresh-open reads. [Corrected measured summary](timing-summary-r2.json):
common ctl Linux monotonic START-to-both-DONE total median **98.052181MiB/s**;
individual five-round C medians **B49.084466 / C52.197064MiB/s**. Common window
includes host relay, launch/result overhead. Cache residency and RPC path were
not observed; no cold/remote-read or qualified3FS performance PASS is claimed.
The first summary included warmup in reader medians; it remains preserved and
explicitly superseded, with common-window values unchanged.

Four services stopped normally: actual bound wait0, eight child/supervisor
incarnations gone, three DFS mounts and UDS gone, complete initial mount inventory
and all11 protected process identities unchanged. [Four final observations](commands/ctl-closed-r2.stdout)
(and a/b/c peers) retain actual receipts. Peak observed allocation **607,477,760B**,
below1GiB; every backing ext4 volume retained its1GiB floor and reservation.

**Failures retained:** wrong host source path on first initial transfer (no guest
input transferred), missing output-parent refusal before writer preparation/I/O
([first writer](commands/a-writer.stdout)), and observerR1 wrongly treating retained
pid/identity/launch receipt indexes as active state ([ctl](commands/ctl-closed.stdout)
and three peers). Explicit fresh result-parent preparation preceded the sole
actual writer/read attempt. No product, criterion or environment repair or unchanged
product rerun. R2 strictly verifies retained indexes against captured incarnation/
argv/lifecycle instead of deleting them; six Linux positive/negative guards passed.
[Small correction](closure-check-r1-to-r2.patch), [guard results](closure-check-r2-affected-guards.stdout).

[Raw commands](commands), [C samples/content checks/lifecycle receipts](runtime),
[relay exits0](relay-result.json), [tool identities](tool-inputs.json),
[reused preparation mapping](reused-preparation-evidence.json),
[external canonical helper/archive recovery](external-artifacts.json).
This packet contains no Python source snapshot, ELF, chunk data, rootfs or TLS key.
SHA256SUMS covers the compact packet; old evidence and failed records are unchanged.

**Next:** affected DFS/current-candidate compatibility or recovery slice on main7e6,
with unchanged standard results reused only after input-impact accounting; then
current colleague trial delivery ledger. Owner workspace metadata attribution,
ordinary performance failures, large/long/complex cases,3FS qualification and
etcd/Redis remain separate deferred items. Overall goal ACTIVE.

[Linux independent packet verification](commands/ctl-independent-packet-verification.stdout)
checks442 pre-final SHA members,48 physical reports,two-reader pre/post content,
five measured rounds each,four wait0/eight gone and unchanged protection.
[Final allocation](final-budget.json) includes temporary proof-packet transfer/extraction.
