# G2.12: real workspace bind ON with remote FUSE — finite functional closure

**Current function: PASS (limited). Measurement: NOT_MEASURED. Performance: PENDING. ON trial delivery: PENDING.** G1 historical 8/8 stays closed; original G2 task counts do not change. This closes subclaims, not full G2.12.

## Product changes and before/after

- [bc835881](https://github.com/lelezi257/dms/commit/bc83588161ef83c77bc932396487dcf809ecb189): successful Home lookup selects the physically observed alias; generic hardlink indexing and held handles stay unchanged. Two meaningful native-disk regressions reproduce old ENOENT and pass after the fix.
- [b80dab66](https://github.com/lelezi257/dms/commit/b80dab66e9d819cad821c9b30edbf97c795c0c3e): FUSE LOOKUP uses the existing OwnerFs freshness TTL for names and attributes. Remote/native zero TTL remains zero; ordinary private local policy stays unchanged. No third-party change, direct-I/O mmap capability or persistence change.
- 0bd runtime: native rename succeeded, remote new-name fresh open failed ENOENT. bc8: new name works, old-name fresh open incorrectly succeeds. b80: new name returns exact data, old name ENOENT. Both failures remain in this index and archived raw evidence. **No throughput/latency improvement was measured or inferred.**

## Closed current subclaims

| Existing ID / subclaim | Current scope and evidence |
| --- | --- |
| G2.12 mount identity | Home B physical ext4 `state/node/ownerfs/root-776f726b7370616365-e1` bound onto its FUSE first-level `mount/ownerfs/workspace`; same dev/inode, nosuid/nodev and distinct FUSE/cover mounts. [Identity](mount-identity.json) |
| G2.12 bidirectional visibility | B native/bind create4KiB, grow64KiB -> C fresh-open full SHA/length/EOF; C shrink/write4KiB and create64KiB -> B bind fresh read. Native rename -> new name read, old ENOENT; remote delete -> native ENOENT. [Receipts](current-operation-receipts.json) |
| G2.12 permissions / errors | uid501 chmod000 read/write EACCES13, restore0600 read; exclusive create EEXIST17 preserves bytes; uid502 read/write denied via0700 parent. Same receipts; not full POSIX |
| G2.12 normal exit | Four runtime lifecycles (bootstrap, ON, remote, Meta) actual wait0, eight PID incarnations gone; owned FUSE/bind/UDS gone, protected process and full original mount inventories unchanged. [Closure](current-closure.json) |
| G2.12 local-file orderly recovery | Reuse the normally stopped current fixture and confirmed renamed4KiB; restart center local-file Meta **and both Nodes**, ON bind and remote reads return original SHA/EOF, source inode/uid/mode preserved, deleted/old names ENOENT and unauthorized UID denied. Three actual wait0. [Recovery](restart-runtime.json), [scope](restart-contract.json), [closure](restart-closure.json). This is **not live Meta-only or crash recovery** |

Current candidate [identity](identity.json): source b80dab66, 158 compiler inputs/map ea809fa7, Linux default release Meta SHA15648a87 / Node SHAb3335fb2. Existing f03 package only carried installer/control scripts; executed binaries were overlaid and independently matched before start. This is not an ON trial package delivery.

Linux fmt,170 OwnerFs unit tests (12 ignored), owner-only lib/bins check, affected Clippy and release bins pass; two pre-existing dead-code warnings remain. Six probe guards pass. [Commands/results](linux-checks.json). The real three candidate runs are the adapter/remote rename regression; no full POSIX or ignored privileged matrix pass is claimed.

## Remaining finite exits and reuse

Next **one centralized explicit ON trial delivery**: identified package, ON config/enable/support/limits and current evidence; installation checks reuse an existing no-compiler environment. Historical host/managed authorization-loss, source/epoch and held-reference drain remain scoped historical evidence; these lookup changes do not affect those paths. A live center-only availability failure still follows current fail-closed policy; it is not covered by orderly recovery. General mixed append/offset/locks/watch, production command issuer/durable ACK, existing-FD instant revocation and expanded topology remain unsupported expansion topics, with prior failures retained.

G2.13 eight small bind kernel-path core performance PASS at >=0.90 ext4 is historical scoped reuse; name lookup changes affect remote FUSE, not the mounted native data path. Do not relabel old metrics as b80 measurements. Then remote G2.14–16 (MooseFS >=1.2 throughput **and** <=0.8 independent latency), DFS one-write/many-read G2.21, ordinary local G2.09–11. G2.21 function/read measurement closed; qualified3FS performance remains pending.

## Evidence, budget and recovery

[Runtime saved-data audit](current-saved-audit.json), [restart audit](restart-saved-audit.json) support functional scope, not new product test counts. Runtime+two failed candidates contain12ERRO/3WARN in10,115B logs, noTRACE; restart logs retain prior events, not additional failure counts. Logs are preserved; do not claim zero errors. INFO,4/64KiB fixture,640MiB aggregate ceiling and ctl512MiB/B-C4GiB reserves are unchanged; [sampled allocation](budgets.json) is not continuous peak.

Raw commands, tool versions/diffs, lifecycle receipts, logs and compact guest state remain outside Git. [Archive index](archive-index.json) / [actual Linux extraction+SHA proof](current-restore.json); [restart archive](restart-archive.json) / [restore proof](restart-restore.json). ELF identities and158-file compiler map are indexed, binary/privateTLS archives excluded. No VM rebuild, volume cleanup, service reset, threshold change or third-party source modification occurred. Protected handoff and acceptance lock are unchanged. [Failure index](prior-failures.json).
