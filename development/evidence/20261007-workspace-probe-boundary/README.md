# Workspace probe test boundary — 2026-10-07

This independent item changes the product/test boundary on main. It does not
reopen G1 historical8/8 or qualify full bind/standard/performance acceptance.
[Plan and coverage](../../workspace-probe-remediation.md) / [configuration and
rootfs recipe](../../../docs/guides/configuration.md).

## Candidate identity

Product source [7e6e00a6](https://github.com/lelezi257/dms/commit/7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d),
157 compiler input map `151a2c6d0de361c1aa3ed2bc9d197102f59563a1ad2a98f56930c981f902c8f2`.
304 compiler/tool final map `f2f04e180bbdaf6f0074cfbae0410d4545adf8de628904c566278bba98d2e3e3`.
Initial source-gate combined map `4a7bf94b51ee4b3f89df6a59d9c834bea58194d1f2c6f56b0d110311ef600002`;
only Python rootfs admission and its new tests changed afterward. Compiler
inputs stayed identical: [scoped reuse](raw/input-reuse.json).

Default-feature release Meta `76a1e34c91697382cba9b3ad1e7bc5a758dd90ca8ededc823fefb9f13708cba2`,
Node `c47be268dc894aac089a020130f7527c58fbc6ada0152ff57f99061c73997fb7`.
Ordinary package `4e5b9e0200e294e72a520085073bb6d226fba1c3c4949f3bbdff5aba09cc86cd`
(14,429,906B; ownerfs,dfs); explicit all-features test example
`4499d7f24be16926a6564f3af7a3fe7b2e5e6bb07a36c5d907bc00cc4740c2c1`.
[Exact package and members](raw/candidate.json). Package/helper binaries and
runtime private inputs stay outside the code tree; their digests bind them.
Old frozen931 candidate and prepared DFS R3 instances remain separate.

## Passed scope

| Item | Result and evidence |
| --- | --- |
| Unchanged helper migration | 100% Git rename to tests/support/workspace_probe.rs, source SHA ed74e6bd…; old product bin absent; core/host/adapter no test name dependency. [Boundary](raw/boundary-proof.json) |
| Affected Linux source gates | 11 gates PASS; 11 ordinary native, 10 config and 8 privileged tests; Owner/DFS checks, strict all-target Clippy, all-features bins/example and helper TERM exit0. [Proof](raw/source-checks/source-proof.json) / per-gate command, raw log and exit alongside it |
| Rootfs dependency admission | Four Linux tests PASS; valid static ARM64 ELF without interpreter accepted, malformed/wrong-arch/dynamic-interpreter/missing-library/unexpected-ldd failures reject. [Tests](raw/rootfs-unit.stderr) |
| Default product build | Actual default-feature cargo build --bins JSON contains exactly afs-meta/afs-node; helper kind example/support path. Historical top-level cache file exists but is not an emitted artifact. [bins proof](raw/default-proof.json) / [plain cargo build proof](raw/plain-default-proof.json) / [raw artifact output](raw/default-build.stdout) |
| Ordinary trial package | Explicit packaged members exclude helper. [Candidate](raw/candidate.json) |

Experimental ON now requires idle_command and identity_command arrays; absent
fields fail explicitly, default OFF/host bind needs neither. The adapter still
needs its configured observer's runtime JSON contract and checks source,
namespace, unique mount, nosuid/nodev before start success and every exec.
Runtime necessary observation is not hidden in tests; only this implementation
of it is a test helper. No third-party source changed.

## Failures and remaining scope

[Initial fmt FAIL](raw/fmt-initial.stdout) is retained; Linux formatting repaired
it before the final gates. Static BusyBox ldd rc1 is retained in
[runtime preparation](raw/runtime-preparation.json); no environment repair.
New Node/container startup, observed identity, seed read and active Node shutdown
PASS in one selected run. Independent postcheck PASS: actual Meta/Node wait0,
four child/supervisor PIDs and container gone, runtime empty, normal FUSE/control
closure, trusted template/config/prefix/Meta-directory and protected objects
unchanged. [Runtime summary](raw/runtime-summary-r1.json),
[actual invocation](raw/runtime-commands/runtime-r1.json),
[result](raw/runtime-text-r1/results/result.json),
[driver checks](raw/runtime-text-r1/results/checks.json),
[one-shot admission](raw/runtime-commands/admission-r1-1.stdout),
[independent postcheck](raw/runtime-commands/postcheck-r1-2.stdout),
[actual waits](raw/runtime-text-r1/results/orderly-final-actual-waits.json)
and [final identity](raw/runtime-text-r1/results/final-identity.json) preserve
that scope. Runtime runc commands include the existing owned `kill --all KILL`;
container disappearance is not proof of graceful init exit or generalized drain.
The standalone helper TERM exit0 is a separate source-gate observation.
132,427,776B allocated (95,739,904B incremental), below512MiB; free7,926,829,056B
above1GiB. One old process,26 old mount rows and38 old ELFs were preserved. Historical results do not automatically pass this new
candidate. Full ON, new-candidate full POSIX, performance, generalized drain,
mixed append/locks/watch, final G2.27 package and official fuser migration remain
separate exits. Next is the prepared DFS small one-writer/two-reader R3 slice;
ordinary performance failures, metadata attribution, 3FS qualification, large
and complex reliability/etcd/Redis retain their priorities and original data.

[SHA manifest](manifest.json) verifies all copied text receipts.
[External artifact mapping](raw/external-artifacts.json) binds the host-side
package/helper and local admission/postcheck/tool archives without duplicating
Python source or rootfs into the current Git tree. Historical evidence links
remain unchanged.
