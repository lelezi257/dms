# DFS automatic transport selection

DFS `auto` prepares the configured RDMA pool and prefers it for chunk replicas
and fixed-version reads. An authenticated canonical unsupported negotiation can
select gRPC before the first data command. Required RDMA refuses that selection.
Business errors and unknown completion never authorize replay through another
transport; a read batch cannot restart after its first RDMA window.

## Validation levels

| Level | Result | Evidence |
| --- | --- | --- |
| Original failure | Old main e8e5e8e with test-only Auto factory graft uses zero RDMA replica bytes instead of8,388,608; exit101 | [Restored-RXE failure](dfs-auto-v75-original-restored/original.log), [graft](dfs-auto-v75-original/graft-test.txt) |
| Local regression | r1 and r2 each pass51 distinct selected cases: original1, peer18, control10, data22 | [r1](dfs-auto-v75-r1/local/original.log), [r2](dfs-auto-v75-r2/local/data.log) |
| Affected integrations | Final Auto RDMA, required RDMA, Auto unsupported gRPC fallback, required unsupported read/write rejection, canonical reply regression and Owner real-RDMA fixture each pass | [Auto native](dfs-auto-v75-r3/related/auto-native.log), [required native](dfs-auto-v75-r3/related/required-native.log), [fallback](dfs-auto-v75-r3/related/auto-fallback.log), [refusal](dfs-auto-v75-r3/related/required-unsupported.log), [canonical](dfs-auto-v75-r3/related/canonical.log), [Owner](dfs-auto-v75-r3/related/owner-native.log) |
| Final stage source gate | Library414/6 explicit or existing environment ignores, contracts65, shared-error4, localAPI9, rootFUSE5; fmt, strict workspace all-target all-feature Clippy, five feature checks and binary build pass | [Library](dfs-auto-v75-r3/full/gate/lib.log), [contracts](dfs-auto-v75-r3/full/gate/contracts.log), [Clippy](dfs-auto-v75-r3/full/gate/clippy.log), [FUSE](dfs-auto-v75-r3/full/gate/fuse.log), [feature](dfs-auto-v75-r3/full/features/dfs-rdma.log) |
| Identity and semantic audit | 54 checks bind143 source inputs, original failure, test-only candidate differences, counts, actual payloads and binaries | [Audit](dfs-auto-v75-r3/audit.json), [script](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-dfs-auto/audit.py) |
| Formal acceptance | NOT_RUN; ENV PREPARING | No formal case is promoted by this batch |

Small changes receive the failing case, related module tests and compilation.
Test-only refinements run selected affected integrations. One complete source
gate runs at the final batch boundary. Full POSIX,8GiB, performance and long
stability tests remain in their planned stages.

## Actual payload and authority

Both native DFS fixtures use the public factories and real product handlers:
two4MiB RDMA READ completions receive a replica and its exact retry, followed by
one75,000-byte RDMA WRITE for an authorized read. gRPC file payload is zero.
The receiver-without-device fixture records8,388,608 gRPC replica bytes and
75,000 gRPC read bytes, zero RDMA payload, correct contents, exact retry and
forged-grant rejection. Required mode rejects both write and read before any
file payload, leaving the destination buffer unchanged. These are real Linux
ARM64 same-VM transport integrations; no new cross-VM Auto Node deployment is
claimed.

The Owner production-client fixture verifies4MiB+17 bytes in each native
direction, sync/cold reopen/content/EOF. It checks the shared negotiation helper
without changing Owner business behavior. [Static review](REVIEW.md) is separate
from executed tests.

## Input and runner identity

The [restored original inputs](dfs-auto-v75-original-restored/before-graft-inputs.json)
cover143 compiler files and match old main before the test graft. The initial
original runner recorded a partial src-only map; that map is retained and is
not used as complete compiler evidence. An absent RXE [environment failure](dfs-auto-v75-original/original.log)
preceded device restoration and is distinct from the reproducible Auto failure.

[Final inputs](dfs-auto-v75-r3/build-inputs.json) match the product tree byte for
byte. Production text is identical across r1/r2/r3; later differences only add
or refine tests. The final full gate and affected integrations belong to r3;
r1/r2 logs keep their original identities. Feature-only checks report dead-code
warnings, including unused fallback fields without compiled RDMA; they are
successful compile checks, not warning-free strict feature lint claims.

Stripped Node SHA: `d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494`.
Stripped Meta SHA: `64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7`.
[Binary manifest](dfs-auto-v75-r3/stripped-binaries.txt) names retained Linux
artifacts. [Runner notes](runner-note.md) record archive/transfer recovery.

## Limits

Canonical malformed-field and startup selection regressions pass. Postdispatch
unsupported injection, later-window unsupported, exhausted fallback deadline,
actual Auto Node startup/deployment faults, posted-DMA cancellation, exceptional
provider reclamation, peak resources and long-run fault matrices are not proven
by this batch. The phase boundary is statically reviewed; static reasoning is
not fault execution. Existing diagnostic catch-all fallback remains separate
from the DFS contract.

No protocol schema, disk format, dependency or module changes. AGENTS and
handoff keep their original hashes; the total delivery goal remains active.
