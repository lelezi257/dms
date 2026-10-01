# Configuration readiness and identified A/B integration

## Contract and repair

The process controller reads listen/mount configuration values using one shared scalar reader. Single-line TOML literal/basic strings may have comments; quoted spaces, `#` and `=` are preserved. Listen parse errors and ports outside0..65535 fail before creating a managed launch. This fixes a real false-readiness failure: Rust Meta accepted single-quoted configuration and logged readiness, while the controller passed a trailing quote to `ss` and constructed an incorrect health URL, then stopped the healthy Meta after20s.

Only `scripts/deploy/afs-processctl` and its existing `selftest.sh` change. No Rust, RPC, module, dependency or durability change. Unicode escapes and multiline deployment strings remain unsupported by this scalar reader; full configuration/DEP matrices remain open.

## Linux regression

[Original failure](integration/attempts/literal-toml-readiness/original-selftest.stderr), [actual Meta log](integration/attempts/literal-toml-readiness/meta.log) and [lifecycle record](integration/attempts/literal-toml-readiness/raw-lifecycle.json) are retained. A new native HTTP fixture accepts either quote style; the old controller fails the literal/comment case under the unchanged2s fixture deadline.

[Final output](integration/controller-final.stdout) and [report](integration/controller-report.json) pass18 labeled Linux groups, including54 existing native lifecycle command records. Two real HTTP readiness runs cover literal/basic strings with comments. Unterminated literal and out-of-range port fail without PID, launch intent or launch directory. Bash syntax checks pass for the three changed/dependent scripts. Shellcheck was unavailable; no Shellcheck claim.

Controller SHA256: `945cba0c6febdd329f50996f3a59586ae9f51be4c29d2ca9b66175b8253dc2a8`. Selftest SHA256: `0085ac461628480202b941f333ab06fe5ac048836954e5009a47f8edb6fbcc5b`.

## Actual A/B proof

[Stage identity](integration/stage.json), [probe result](integration/probe.json) and [Root independent verification](integration/root-verification.json) bind v51 Rust binaries from the [coherent Linux gate](../20261001-shutdown-signal/source/qualified-linux/report.json), current controller, configs, workers and Linux process incarnations.

| Lane | Runtime | Processes | Ports |
| --- | --- | --- | --- |
| A | `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v51` | Meta722421,Node722590 |18280..18283 |
| B | `/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v51` | Node131467 |18284..18285 |

Both use guest ext4,memory Meta,R1,gRPC/TLS and separate OwnerFs/DFS mounts. A uses verified hard links to the completed isolated v51 binaries to conserve ext4 space; no binary contents are overwritten. B receives the same immutable Node binary. Existing v48 and v45 runtime/data remain preserved. The new A/B processes remain live for subsequent functional work.

- [Consistency report](integration/consistency/report.json):10/10 steps,30 commands,zero failures,qualified cross-mount.
- [OwnerFs locks](integration/ownerfs-cross/report.json):7/7 steps,29 commands,zero failures.
- [DFS locks](integration/dfs-cross/report.json):7/7 steps,29 commands,zero failures.
- Locks retain35s minimum blocking wait,55s child timeout and the source-identified30s peer request cap.
- Root checks248 product PID/hash/start-tick/executable records, all143 unchanged Rust inputs, live config/controller/worker hashes, exact FUSE mounts and current readiness. Preserved v48 process incarnations match before/after.

Node SHA256: `97d735243d741ef98d917f05a965c9e02703b3b05468ba42e7b1d7c5f2c38e31`; Meta SHA256: `00cb99fe46f20024ab3b476978dd0e16d9e8a2c0535b95d5c8febc3aa863cf2d`. The full pjdfstest run retained in prior evidence still qualifies v48 only, not these binaries.

## Preparation and verifier attempts

[Initial preparation](integration/attempts/initial-stage-permission/failure.json) is BLOCKED: root-owned staging directories prevented Lima copy before product launch. Directory ownership was corrected only in newly created paths. A one-time host continuation then had an indentation error before guest mutation; its original script/note remain in that attempt. Neither is counted as product failure or release evidence.

[Initial verifier selection](integration/attempts/root-verifier-record-selection/failure.json) is INCONCLUSIVE: generic PID/hash traversal included Linux Python test-worker identities. The corrected verifier selects explicit Node/Meta roles, checks every such identity and records worker script hashes separately. Product probes were not rerun to replace these observations. The final verifier and report are retained.

## Reproduction and remaining gates

[Orchestrator](reproducers/integration-v51.py) is a research-layout host controller; place it under `experiments/afs-acceptance/` with the accompanying probes under `probes/`. It needs the fixed Lima identities, preserved v48 configs and v51 qualified binaries/TLS. Preparation refuses existing runtimes; it never resets old state. The one-time continuation/start scripts under `integration/` record this completed attempt and are not general installer entry points. All real file I/O and Rust checks run in Linux; macOS only orchestrates/copies/hashes.

This does not prove full installation, backend/R=N/RDMA fault matrices, complete POSIX release coverage, performance or soak.69 formal cases remain NOT_RUN; environment PREPARING; goal active. No executable binaries or TLS private keys are included. `docs/handoff.md` is unchanged. [Manifest](artifacts.json) records raw artifact bytes/SHA256, distinct from semantic assertions.
