# Remote standard-suite identity validation

Date: 2026-10-01. Linux ARM64 A/B development lane, memory Meta, DFS R=1. These results qualify the remote test driver and its short matrix, not the complete release contract.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Driver and identity Linux selftests | 48 PASS | [Raw selftest](linux-selftest.log) |
| Actual B DFS pjdfstest with A Meta identity | Four files / 241 checks PASS; zero unexpected failures, skips or TODO | [Host proof](host/artifacts/std-01-pjdfstest/proof.json) |
| Raw guest TAP and accounting | 236 discovered files; four selected, 232 not run in smoke | [Worker accounting](worker/artifacts/std-01-pjdfstest/tap-accounting.json), [TAP](worker/artifacts/std-01-pjdfstest/pjdfstest.stdout.tap) |
| Retrieved host and worker artifact hashes | All seven entries match exact file bytes; worker proof matches the host record | [Verification](manifest-verification.json) |

## Contract

Before running the suite, the host checks A Meta and B Node process SHA, start time, boot identity, configuration, TLS digests and mount identity. A Meta must own the expected listening endpoint; B Node must use that endpoint. Separate worker identities are required by default. The fixture is inside B's actual AFS mount and worker evidence lives on guest ext4. Post-run checks require stable processes, configuration, TLS, mount and evidence volume.

The worker returns complete raw TAP accounting. The host retains the original worker output in a local artifact and binds remote artifact paths to their manifest entries. Each manifest excludes its own `proof.json`; that final proof is embedded in the host record. This avoids a self-referential hash while retaining exact raw evidence hashes. Missing identity, malformed output, missing evidence or timeout fails closed.

Node SHA256: `143294f76870794aab05f7f314287622367914f98b329c98b5fccd3a60a88faf`.
Meta SHA256: `ffe7268031438318de65ee999e4872846974784141fba4d85ad451865a958d01`.
A Meta PID576299; B Node PID8811. These are observed run identities, not portable configuration.

All suite operations and selftests run on Linux. macOS performs host orchestration, copying and hash comparison only. No private key, executable or VM state is published. Full STD-01, required backend variants and the formal 69-case release matrix remain unqualified. The separate v37 DFS cancellation failure remains open.
