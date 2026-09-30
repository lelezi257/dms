# Remote owner open identity and provider lifetime

Date: 2026-10-01. Linux ARM64 candidate v43, memory Meta, two independent A/B FUSE mounts. This is a development milestone, not release acceptance.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Original owner-open ACK-loss behavior | Regression FAIL | [Raw log](linux-before-ack.log) |
| Original provider release order | Regression FAIL | [Raw log](linux-provider-before.log) |
| Release without waiting for admitted reads | Regression FAIL | [Raw log](linux-inflight-before.log) |
| Linux source gate | 319 library PASS, two explicit environmental ignores; 57 interface / 4 shared-error / 5 privileged actual FUSE PASS | [Gate log](linux-qualified-gate.log) |
| Formatting, strict all-target/all-feature Clippy, feature checks and build | PASS | [Gate log](linux-qualified-gate.log) |
| Compile identity | All143 host/Linux inputs match | [Host](host-source-hashes.json), [Linux](linux-source-hashes.json) |
| Actual A/B consistency | 10/10 PASS | [Raw report](v43-consistency/report.json) |
| Actual DFS / OwnerFs cross-node locks | 7/7 each PASS, original35-second wait /55-second interrupt bounds | [DFS](v43-dfs-locks/report.json), [OwnerFs](v43-owner-locks/report.json) |
| Runtime identity | Node/Meta hashes match at every captured process record | [Verification](runtime-identity-verification.json) |

The original v42 runtime had two failures out of ten consistency scenarios: remote-owner write/close followed by read, and a fresh read after handleless resize and writer close. [Original raw report](v42-consistency-failure/report.json) and compressed A/B Node logs preserve the failure. Both exact scenarios pass on v43 without assertion polling or increased timeouts.

The first v43 controller invocation omitted the expected B Node identity and failed qualification before running any scenario. [Preflight failure](v43-controller-preflight-failure/report.json) is retained. The qualified invocation supplies all three expected process identities. An initial source test fixture had an unwired peer read source; its failure is preserved separately. The final fixture shares the owner's chunk store/read engine to isolate provider lifetime.

## Contract and RPC cost

Each Open has a caller-known nonzero sequence scoped to caller and owner process sessions. Active exact replay returns the same handle; altered request bodies and retired replay are rejected. A lost or malformed Open reply retains an exact cleanup identity, reserved capacity and the existing Release retry. Release alone accepts an empty opaque handle with complete admission identity. Ordinary read/write handles still require the owner-issued opaque handle.

Caller-to-owner route admission is serialized through the reply or cleanup retention. Owner state uses active identities and bounded route high-water marks instead of permanent per-close tombstones. Authoritative caller-session retirement removes the old owner-side route; unknown session queries retain it.

Remote same-mount read/getattr uses a shared provider I/O guard. Close stops new admissions and removes/replaces the provider before owner Release, then waits for that exact provider's admitted I/O. Replacement liveness is rechecked while publishing under the provider map lock. Network calls do not hold provider/handle/lifecycle mutexes.

No RPC method is added: ordinary Open remains one request, and O_WRONLY retains its separate read companion admission. Meta schema, OwnerFs protocol, module names and directories are unchanged. Control/data owner handles and Open requests gain field9, open_seq. All participating Nodes need a coordinated protocol upgrade; old zero-sequence peers are rejected.

## Identity and limits

Node SHA256: 9c311471c4e4bcbd605b326e5955c12399f659ba5176cc1f5c7fe83d589fa12e.
Meta SHA256: c261dfe3de628e03bc0ece84d645562243ebaf5a4997f6b8097943edb4c55ea8.
A Node704806 / Meta704776; B Node66113. Runtime roots end in p2-memory-lane-v43 and p2-memory-peer-v43. Raw reports bind Linux boot IDs, mounts and process start ticks.

Pending caller cleanup after an owner-session replacement, a global writeback/shutdown budget, full fault matrices and persistent-backend parity remain unqualified. Provider wait_idle has no aggregate timeout; production read/getattr RPCs have their existing timeout, but this does not prove a global shutdown bound. The full upstream suite and release gates are not qualified by these short runs. Formal69 remain NOT_RUN; ENV remains PREPARING.

The [earlier full remote attempt](../20261001-remote-full-timeout/README.md) remains BLOCKED at the original1800-second limit. Raw v42 and test-injection failures are retained separately. Handoff documentation is unchanged.
