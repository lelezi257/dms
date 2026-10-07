# Current trial workspace data and selected core performance exit

2026-10-07. **Fact: G2.13 complete within current7e6/C1/eight selected small
core cases.** G1 historical8/8 stays closed. G2 now has11 limited completions,
two ordinary performance FAILs, one bind functionality item in progress and13
pending items. The overall goal stays ACTIVE.
[Predeclared data plan](../../workspace-bind-data-current.md).

Product7e6e00a6/157 compiler map151a2c6d, published ordinary packagec3bb5a30,
default-feature Meta76a1e34c/Nodec47be268. [Exact candidate](candidate.json),
[compiler inputs](compiler-inputs.json), [executed tool map](tool-inputs.json),
[rootfs map](rootfs-inputs.json). No Rust/vendor/C change or rebuild. Historical
931 timings retain their source/profile identity and are not inherited.

## Current independent data result

| Case | Five paired speed/ext4 median | Range | Result |
| --- | ---: | ---: | --- |
| ON64MiB sequential write+fsync | 0.969905 | 0.668950–1.093036 | PASS, preset>=0.90 |
| ON64MiB fresh-open read+close | 0.965910 | 0.845987–1.704626 | PASS, preset>=0.90 |
| OFF write diagnostic | 0.728219 | 0.631512–1.032956 | FAIL, retain/defer tuning |
| OFF read diagnostic | 0.457996 | 0.404028–0.586780 | FAIL, retain/defer tuning |

Both cohorts use C1,64MiB,1MiB blocks,pattern90, one warmup and five alternating
paired measurements. The fixed PR43 /io C payload measures write+fsync then
fresh-open read+close inside ordinary official-runc1.5.2 containers. Cache is
unobserved; repeated write-then-read results do not prove cold media or physical
durability. Five paired reference-wall/experiment-wall ratios are distinct from
quotients of throughput medians. [All rounds/commands](runtime),
[absolute throughput supplement](throughput-supplement.json).

The independent host switch alone covers the physical Home ext4 directory onto
OwnerFs FUSE's first-level workspace. Ordinary runc binds that host view;
the legacy container adapter stays OFF. [Before/after](runtime/binding-before.json),
[ON container identity](runtime/on-experiment-container.json), four original OCI
`*-spec.json` files and clone/input hashes retain actual dev/inode, namespace,
unique mount, nosuid/nodev, UID/GID501, readonly rootfs, zero capabilities and
noNewPrivileges. The frozen historical helper in this test rootfs supplies
idle/identity; it is not in the ordinary product package or a bind-core dependency.

189 driver checks and258 independent Linux checks PASS. The collector re-read48
actual C payload outputs and48 raw metric scrapes, recomputed complete pairs and
fixed thresholds, and bound the current tools/package/ELFs and four container
identity outputs. All six OFF experiment windows have positive read/write
callbacks; ON read/write callbacks are zero. Other counters are retained.
[Independent proof](independent-post.json), [actual invocation](commands/post-r1.command.json),
[output](commands/post-r1.stdout), [collector adaptation](collector-current.patch).
15 affected Linux tool guards passed; their evidence is explicitly a
[captured native tool-response summary](linux-guards-observed.json), not a raw
stderr log. Historical unchanged guards and changed inputs are separately bound
by [tool reuse accounting](tool-reuse.json).

Four service actual waits0 were saved, including OFF before ON startup cleans
closed lifecycle directories. Eight service/supervisor plus four OCI PIDs are
gone, TERM/stopped/deleted containers leave an empty runtime, and normal FUSE
mount closure passed. Detached runc cleanup is not a container-init wait0 claim.
One protected process and all26 prior mount rows remain exactly unchanged.
[OFF waits](runtime/off-actual-waits.json), [ON waits](runtime/on-actual-waits.json).
Peak245,153,792B<256MiB, independent post allocation98,627,584B and
free7,484,760,064B>1GiB. No dependency/VM repair, resize or score repeat.

Full small logs7274B remain in [runtime/service-logs](runtime/service-logs):
16 INFO/19 ERRO/2 WARN. Negative root/file lookup and non-user xattr errors,
plus unknown FUSE opcode52 warnings, are retained in [log accounting](log-accounting.json).
This is not zero-error-log/full xattr/opcode qualification.

## Combined eight-case exit

The [combined ledger](combined-core-acceptance.json) binds this new data pair to
the already-published [same-candidate six metadata cases](../20261007-workspace-bind-metadata-window/README.md):
create/write/close0.994725, stat0.964347, read/close0.966917, readdir1.032311,
rename0.992779 and unlink0.984438. All eight medians meet the predeclared>=0.90;
metadata evidence is reused without repeating tests or changing its original
single-case conclusion. The actual G2.13 exit is same-candidate core data and
metadata performance/content/selected semantics. OFF diagnostic FAIL is allowed
by both plans and remains FAIL.

This combination closes that limited performance item. It does not close G2.12
full ON, Issue42/PR43's full IO shapes/combinations, mixed append/locks/watch,
general lifecycle/drain, full POSIX, random/concurrent/cold/large cases, physical
durability or qualified MooseFS/3FS parity. The metadata plan's original warning
against promoting its one small case remains unchanged; production ON stays
unqualified/default OFF. Broad goals remain open.

[External canonical archives/SHA/recovery](external-artifacts.json) retain all
original bytes outside source; no Python/C/Rust snapshot, ELF/rootfs or TLS key
is copied into this packet. SHA256SUMS covers the portable text packet.
**Next:** current7e6 OFF standard-suite applicability/reuse audit; run only
actual changed-path gaps, then update the current core-performance delivery
status. Ordinary performance tuning/qualified comparators and larger/complex
backend cases retain their separate priorities.

**Verification failure retained:** the first independent packet verifier incorrectly expected the prior metadata packet to use SHA256SUMS. Its actual immutable format is manifest.json. The read-only verifier was corrected to check every original byte/SHA entry and complete coverage; no product, environment, criterion or historical evidence was changed, and no benchmark was repeated. [Original failure](commands/linux-evidence-verification.stderr), [exact correction](verifier-manifest-correction.patch).

[Final independent Linux packet proof](commands/linux-evidence-verification-r2.stdout):1355 checks including579 pre-final members,157 compiler inputs,9 executed tools, all original metadata manifest entries and the combined eight-case exit. Final SHA manifest adds these proof receipts without changing runtime results.
