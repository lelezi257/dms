# Current checkpoint validation

**PASS — combined source and tools only.** [Structured receipt](validation.json), [248 input hashes](r3/inputs.json) and individual `r3/*.command`, `*.log`, `*.exit` are included in this Git snapshot. The ARM64 Linux run used locked offline dependencies and separate Linux Cargo output. All18 command exits are0; input equality was verified after the run and again when producing the receipt.

| Check | Actual result |
| --- | --- |
| Compiler/tool identity | 154 compiler inputs +94 tool inputs; map `e71dea8fdc2a1f674aa21418c8f076d6bb7b0264897c5d5e97a1390f0ee78aa0` |
| Compiler map | `b850fcf4ab8882f9428dc1094a6e52647ddea465a038666467c6f1542487a068` |
| Library | Outer547 passed/12 ignored;7 isolated child subprocess runs separately recorded |
| Contracts | 114 passed/13 ignored |
| Actual privileged FUSE | 8 passed |
| Error / Local API | 4 /9 passed |
| Formatting / features / lint / build | fmt; fivefeature configurations; strict workspace/all-target/all-feature Clippy; all-feature binaries built |
| Acceptance driver regressions | 316 discovered,315 passed,1 skipped; optional old frozen STD-01 identity fixture absent |
| Deployment script regressions | trial TLS/config; selfcheck reference bytes; reproducible package generation/umask checks passed |

The package reproducibility test uses controlled fixture binaries. It verifies script behavior, not independently installed current-release equivalence. Library/contract ignored tests remain explicit and are not counted as passes. Current full pjdfstest/LTP/FSx, installed multihost/local-file recovery and formal ext4/MooseFS/3FS performance remain NOT_RUN. Production native remains DISABLED.

Current built ELF SHA256 (binaries themselves are not committed):

```text
38f0e76a5b4c4afc3efdde3ee7bfa96b0e7e01a0403d556343ec385a86b20d45  afs-meta
04b68193d7cdd04dea8c861f07e47336be05281de11a91e9ea8f3890123d1900  afs-node
```

`historical/` contains byte-preserved older structured results with [version boundaries](../../../current-checkpoint.md#historical-evidence); those are not measurements of this candidate.

## Preserved attempts

Preparation R1 encountered `OSError: [Errno 95] Operation not supported` while the input scanner touched an ignored runtime-evidence socket, before testing. The scanner now prunes runtime/output directories. This is the orchestration observation; no original R1 stderr file was captured.

R2's14 Rust checks passed, but the [driver log](r2-failure/acceptance-drivers.log) records316 tests/onefailure/oneskip: STD-05 accounting is READY while an old assertion demandedTODO. The fixture assertion is corrected to require READY, the exact accounting command and product status NOT_RUN; no product pass was fabricated. Original input/log/command/exit are retained. R3 is the successful post-correction18-command run.

The receipt-generation step initially tried writing to the read-only Linux source share and failed after reading/verifying results. The summary was then generated on guest ext4 and copied to this directory. That artifact-copy correction did not change validated inputs or rerun/override a failed product check.
