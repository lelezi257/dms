# Current default-OFF trial — 2026-10-07

Current product [7e6e00a6](https://github.com/lelezi257/dms/commit/7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d),
157 compiler inputs/map `151a2c6d0de361c1aa3ed2bc9d197102f59563a1ad2a98f56930c981f902c8f2`.
[Predeclared slice](../../current-trial-7e6-slice.md), [candidate](candidate.json),
[commands](commands/installed-recovery.command.json), [result](results/result.json),
[independent postcheck](commands/runtime-postcheck.stdout).

| Classification | Scope and evidence |
| --- | --- |
| **Current PASS** | Ordinary Linux ARM64 package built twice from the same existing release ELFs, both 14,429,532B/SHA `c3bb5a30a1dd74a4b00d45388298e0b0628bb84d8aefbddf29ed2ec633b5f8e6`; [manifest/members/reproduction](package-reproduction.json). No source/test probe/rootfs/private keys in the package. |
| **Current PASS** | One fresh no-compiler Linux/ext4 installation on afs-g2-micro; both workspace switches OFF, OwnerFs+DFS/R1/gRPC/local-file. 43 driver checks include configuration/TLS admission, 64MiB full-content write/sync/close and fresh read/SHA/EOF after normal Meta-only restart. [Actual result](results/result.json), [before](results/recovery-before.json), [after](results/recovery-after.json). |
| **Current PASS** | First Meta actualwait0 preserved before next launch; final Meta/Node actualwait0. All six child/supervisor PIDs gone, FUSE/control sockets normally closed; Node incarnation and both mounts unchanged across Meta restart. [First wait](results/first-meta-actual-waits.json), [final waits](results/final-actual-waits.json), [independent closure/inventory](commands/runtime-postcheck.stdout). Original one process/26 mount rows/40 ELF identities unchanged. Allocation234,819,584B<512MiB, free7,691,952,128B>1GiB; FUSE mount tree excluded from capacity walk. |
| **Current tool PASS** | Seven original runner checks, five new wait-receipt rejection checks, three new OFF/configuration checks on Linux. [Tool identity](tool-identity-r3.json) and raw [baseline](commands/baseline-guards-r1-1.stderr), [wait](commands/affected-wait-guards-r2-1.stderr), [configuration](commands/affected-config-guards-r3-1.stderr). This is tool coverage, not POSIX qualification. |
| **Historical PASS, scoped reuse** | [e925 release Owner pjdfstest](../20261007-owner-standard-reuse/README.md) and [0891 local-R1 DFS pjdfstest](../20261007-standard-reuse/README.md), each236 files/8819 checks/28 upstream TODO; Owner LTP6/short FSx were dev-only and DFS selected LTP6 retains its recorded profile. 931 workspace data and DFS R3 one-writer/two-reader results stay under their own binaries and commands. [Static impact map](impact-map.json) corrects the old claim that all FUSE/lifecycle code was unchanged: counters and Node lifecycle changed since6d, while core business/Meta/storage semantics remain equivalent or unchanged. Fresh installation/recovery above covers the affected ordinary path; no full standard or performance rerun is claimed. |
| **Pending** | Full G2.27 selected performance exit, full ON qualification, current7e6 R3 cross-node runtime, formal comparator parity and broader reliability. G1 historical8/8 stays closed. No new product FAIL/environment blocker in this bounded run; ordinary performance/metadata counter failures and official fuser API gap remain recorded separately. |

Meta ELF `76a1e34c91697382cba9b3ad1e7bc5a758dd90ca8ededc823fefb9f13708cba2`;
Node ELF `c47be268dc894aac089a020130f7527c58fbc6ada0152ff57f99061c73997fb7`.
No Rust/vendor change or rebuild in this slice. [Probe boundary and Linux build/runtime
proofs](../20261007-workspace-probe-boundary/README.md) remain unchanged.

[Trial instructions](../../../docs/guides/trial.md) and fixed
[prerelease entry](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a)
identify this archive; public asset availability is confirmed by a publication receipt,
not inferred from the URL. Existing6d release remains historical.
[External artifacts](external-artifacts.json) bind the host archive and observer;
[SHA manifest](manifest.json) covers the compact text packet. Canonical maintained
Python tools stay in development/acceptance; no full tool snapshots are copied here.
The pre-run entry-map-r2 and impact classification preserve their original pending state;
the current PASS above is the subsequent runtime result.

Next: resume workspace metadata callback attribution with a narrow observation fix;
retain ordinary performance FAIL data and defer targeted tuning. Large/complex,
3FS qualification topic, etcd and Redis retain their agreed order.
