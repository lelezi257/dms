# AFS Acceptance Experiment Manifest

This directory holds the first-stage AFS acceptance manifest derived from [docs/acceptance.md](../../docs/acceptance.md).

## Files

- `cases.json` records contract case IDs, applicability, smoke/full boundaries and driver readiness. Formal driver registration remains TODO. Case status stays `NOT_RUN`; results are separate immutable artifacts.
- `acceptance.lock.json` remains `PREPARING`: the image and four guest inventories exist, and network/RXE probes have partial evidence. Comparator mounts, complete reference suites, faults, and release identities are still required before freezing it.
- `runner.py` dispatches registered drivers and verifies structured results and matrix coverage. `test_runner.py` exercises conservative failure handling; runner self-check success is not product acceptance.
- `environment.py` evaluates hash-bound preparation observations. It reports missing or unsupported predicates and does not freeze the lock or change case status. Full dispatch requires environment qualification in addition to actual contract/source/binary/runner/manifest identity; generic PASS fields cannot substitute for proof.
- `results/` is reserved for future runner output. Result files should be immutable run artifacts containing commands, logs, JSON/JUnit, seeds, environment identity, success watermarks and raw evidence. Results must not be written back into `cases.json`.

## Active Scope

The manifest contains 69 active first-stage cases:

- `ENV-01` environment gate.
- `STD-01` through `STD-05`.
- `FUN-01` through `FUN-13`.
- `DIST-01` through `DIST-08`.
- `PERF-01` through `PERF-08`.
- `REL-01` through `REL-14`.
- `RDMA-01` through `RDMA-05`.
- `OPS-01` through `OPS-07`.
- `DEP-01` through `DEP-08`.

`REL-15 duplicate-active-meta` is present only as a reserved, non-active ID for later Meta HA design. `TODO-01` through `TODO-07` from the contract are not active manifest cases and do not count toward the first-stage release gate.

## Stages

The stage order follows the contract:

1. `environment` prepares the VM/RXE/baseline gate and deployability prerequisites.
2. `ext4baseline` freezes reference suite behavior and accounting.
3. `short` runs fast representative checks with small data.
4. `semantic`, `distributed` and `fault` cover the product behavior matrix.
5. `perf` runs paired baseline comparisons.
6. `full` runs long stability and full regression gates.

Long 900-second FSx runs, eight-hour soak and 8 GiB performance/basic-IO data are only full gates. The smoke fields keep smaller equivalent boundaries so runner development can validate wiring without pretending those reduced runs satisfy the full gate.

## Matrix Notes

Backend applicability is explicit per case:

- General POSIX, semantic, reliability, deployment and operations cases usually apply to both `OwnerFs` and `DFS` across both `etcd` and `Redis`.
- OwnerFs-only cases are limited to Home/workspace behavior, including `DIST-01`, `REL-12` and `OPS-07`.
- DFS-only cases cover replica, layout, Chunk, repair and multi-source semantics, including `DIST-02` through `DIST-05`, `DIST-08`, `REL-03`, `REL-07` and `REL-09`.
- `RDMA-01` is an environment preflight and is not product file-data-path proof; `RDMA-02` through `RDMA-05` apply to the relevant OwnerFs remote and DFS peer/RN data paths.

Standard suite exclusions are not incremented by this manifest. Exclusion counts belong in the future lock/results evidence and must be fixed before suite execution.

## Execution Boundary

The runner exists, but formal driver bindings and the full environment are still being prepared. `drivers/standard.py` implements pjdfstest execution, accounting and guarded remote orchestration; its Linux selftests and captured development runs are recorded in product `development/evidence/`. The STD-01 manifest entry remains TODO until its release environment and backend bindings are complete. TODO registrations return BLOCKED. An unfrozen lock blocks full acceptance. Development probes outside the manifest retain their own scope and identities, never substituting for a product case PASS. Concrete runs belong in `results/`; do not overwrite case statuses with preparation claims.

## Preparation evaluation

Run in Linux ARM64 with copied immutable inputs:

```bash
python3 environment.py --lock preparing.lock.json \
  --bundle bundle/bundle.json --contract bundle/acceptance.md \
  --output preparation.json
```

The bundle has `artifact_references`, a map of relative raw artifact paths to
SHA256 digests. Its observations include `host.json`, `lima-after.jsonl` and the
four guest inventories. `contract` binds a relative `path` and `sha256` to the
actual acceptance file; the lock's `contract.sha256` binds the accepted content.
Host initial reserve is UNKNOWN if no observation from before provisioning is
available; current free capacity cannot substitute for it.

The partial evaluator checks selected metadata and hash-bound network and verbs exchanges,
and retains separate outstanding semantic prerequisites. It cannot produce
complete ENV-01 qualification: backend restart, reference
accounting, actual comparator mount I/O, frozen inputs and run contracts still
need dedicated validators and live checks. This limitation is not a change to
the acceptance standard. A comparator mount check and its later fair durable
performance qualification have separate evidence and results.

## Standalone verbs preparation probe

`probes/env_verbs.py` collects an independent Linux ARM64 stock `rping`
exchange. It pins the installed rdma-core binary and records guest, process,
provider, route, GID, MTU, command and raw-log identities. Start a bounded
server before its explicitly source-bound client, then use `evaluate-pair`
on their JSON records. Linux regressions are in `probes/test_env_verbs.py`.

The probe checks complete READ and WRITE echo contents and successful verbs
completions. SEND carries 16-byte MR descriptors and go-ahead control; paired
descriptor digests are reconstructed from advertised/received fields, not a
packet capture or a bulk SEND content test. Installed provider hashes do not
attest a dynamically loaded provider. Cleanup/resource observations and the
full required topology belong in the collected run evidence.

The [cross-VM preparation evidence](../evidence/20261001-verbs-preparation/README.md)
contains exact invocation, twelve directions, a boundary payload and an
absent-listener failure. The optional environment bundle block
`verbs={prefix:"verbs"}` supplies these immutable files to the dedicated
`cross-vm-verbs` predicate. It verifies sources, reports, raw contents,
commands, guest/process identities, descriptors and cleanup rather than
trusting a PASS receipt. Missing evidence remains BLOCKED.
Exact client/server and absent-listener commands must cover their endpoint
capture intervals. Record matching allows an explicit 50 ms host/guest clock
skew; this does not qualify the separate ENV clock-accuracy requirement.
The [consumer evidence](../evidence/20261001-verbs-predicate/README.md)
records its regression and full-dispatch guard checks. This is preparation,
not a formal case driver. ENV remains PREPARING; product RDMA
file I/O, fallback, lifetime and security have separate acceptance gates.

## Standalone network preparation probe

`probes/env_network.py` provides bounded fixed-IP TCP/UDP/mTLS echo checks and
TLS rejection checks in Linux ARM64. `probes/test_env_network.py` covers framing,
failure classification and socket/thread cleanup. It uses isolated probe
credentials and ports; it does not authorize product RPCs or qualify RDMA.
Linux invocation and captured identities are documented in the
[network preparation evidence](../evidence/20261001-network-preparation/README.md).
The environment bundle can reference these observations:

```json
{
  "network": {
    "prefix": "network",
    "probe_source": "network/probe.py",
    "commands": "network/commands.jsonl",
    "fault_source": "network/fault.sh"
  }
}
```

Every consumed file must be listed in the bundle's `artifact_references` with
its SHA-256. The evaluator checks supported probe source, four guest identities,
twelve ordered pair exchanges, actual endpoints, TLS rejection details, command
identity/order, directed DROP counters, exact rule restoration and listener
cleanup. Missing proof is BLOCKED; altered or contradictory proof is FAIL.
Generic PASS summaries are insufficient. Original wire observations retain
their frozen source identity when reused; a later source change cannot relabel
them as a fresh run.

`fault_source` binds the supported declared fault recipe. The historical run
did not capture its executed-file hash at injection time; this field cannot
retroactively provide that attestation. Actual directed DROP counters, failed
exchanges, command order and final rule restoration supply the fault evidence.

This predicate proves the recorded TCP/UDP/mTLS and directed-fault preparation
slice. Recovery by the main flow is separate from independent watchdog recovery;
a completion marker does not prove the latter. Full ENV, product RPC trust,
cross-VM verbs and formal reliability require their own evidence. All remaining
prerequisites must pass before the environment can be qualified.
