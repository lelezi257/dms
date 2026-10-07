# Current trial DFS R3 content and normal Meta recovery

2026-10-07. **Fact: bounded content/three-copy/normal-Meta-recovery PASS.**
G1 historical8/8 stays closed; G2 major counts and qualified3FS parity stay open.
[Plan and affected coverage](../../dfs-r3-current-recovery.md).

Product [7e6e00a6](https://github.com/lelezi257/dms/commit/7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d),
157 compiler map151a2c6d, published ordinary packagec3bb5a30,
Meta76a1e34c/Nodec47be268. [Exact identities](candidate.json),
[compiler inputs](compiler-inputs.json), [executed tools](tool-inputs.json).
No Rust/vendor change or rebuild. Historical931 five-round timings and historical
standard suite results retain their original version/scope; this is fresh7e6
runtime evidence, not an inherited PASS.

Four existing ARM64 Linux/ext4 VMs were admitted before services started:
dependencies, binaries, tools, TLS, print-config, ports, RAM, protected inventory,
exact3 synchronous durable copies/minimum3 node/configured-domain identities,
local-file Meta/gRPC and both workspace switches OFF.
[Four-role admission](four-role-admission.json), [raw commands](commands).
Three node/VM identities share one physical host, not three physical failure zones.

A wrote64MiB,64 distinct counter-prefixed1MiB blocks, fdatasync/close and directory
fsync. Full SHA `4c47e859a6831026ba9367262afb1e1803d11abc30f6f3d996ceea829af89324`
and EOF passed. [Writer](writer-confirmed.json).
Before B/C reads, the independent onsite observer checked16 different4MiB chunks,
three distinct Ready/Durable CopyRecords each and48 original physical chunk
byte patterns/SHA with matching durable catalogs. [Before proof](before-restart-replica-proof.json).
No192MiB chunk archive; CopyRecords are derived receipt facts, not complete original
ReplicaAck objects. B/C each passed one fresh-open C read plus complete pre/post
SHA/EOF. [B](before-restart-reader-b.json), [C](before-restart-reader-c.json).

Meta alone stopped with an exact saved actual wait0; the same installed binary,
config and local-file store restarted once. All three Node incarnations, FUSE
mount rows and UDS identities remained identical. [First exit](first-meta-closed.json),
[restart identity](meta-restart-identity.json). Afterwards, the same layout/version,
CopyRecords and48 physical copies were checked again, then new B/C opens passed
complete content/EOF. [After proof](after-restart-replica-proof.json),
[B](after-restart-reader-b.json), [C](after-restart-reader-c.json).

Final normal stops prove five actual service waits0 including the first Meta,
ten owned child/supervisor PIDs gone, three DFS FUSE/UDS closures, exact original
mount inventories and11 protected process identities unchanged.
[Closure](closure-summary.json). Peak observed allocation597,450,752B<1GiB;
every backing ext4 volume retained its1GiB floor/reservation. [Budget](after-restart-budget.json).

**Failure retained:** the first post-restart observer wrongly required two
accumulated lifecycle directories. The installed processctl intentionally removes
a verified closed generation when launching its successor. The first actual wait0
was saved before restart; R2 requires that exact saved receipt and a new bound
incarnation, while still rejecting extra/foreign live directories.
[Original failure](commands/ctl-second-meta-capture.stdout),
[read-only correction](closure-r1-to-r2.patch),
[12 Linux observer guards](commands/restart-observer-guards.stdout).
No VM/dependency/product repair or repeated product restart. Seven current-driver
and14 fixture Linux guards passed; the earlier seven observer guards remain scoped
to their original, incomplete retention assumption.

All four full logs are small and retained in [logs](logs), with full archives and
durable metadata outside Git. [Accounting](log-accounting.json):14 INFO/5 ERRO/19 WARN,
including negative dentry lookups, unsupported non-user xattr/ioctl and one retry
during Meta downtime. This is not a zero-error-log or full xattr/ioctl PASS.
[External hashes/recovery](external-artifacts.json), [runtime receipts](runtime),
[summary](summary.json), SHA256SUMS bind this packet. No Python source snapshot,
ELF, chunk bytes or TLS private key is copied into the evidence tree.

This is ordinary normal-restart coverage, not crash/complex reliability, full POSIX,
performance parity, cold/remote-path proof or completeG2. Single-read C timings
are retained as functional observations; no new five-round score.
**Next:** current7e6 workspace64MiB paired data read/write, keeping931 results
historical. Ordinary performance FAIL data, qualifiedMooseFS/3FS, large/long cases,
complex reliability and etcd/Redis remain separate/deferred. Goal ACTIVE.

[Independent Linux verification](commands/linux-evidence-verification.stdout):938 checks including847 pre-final SHA members,157 compiler inputs, exact executed tools, current copies/content and five actual waits. Final manifest adds these verification receipts without changing runtime results.
