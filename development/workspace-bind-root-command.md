# Workspace bind RootCommand receipt and refusal

Decision: one small host-ON functionality item; default OFF, G1 historical8/8 and G2 aggregate counts remain unchanged. This is separate from physical Home identity polling, registration heartbeat, full revoke/ACK and performance. Core stays in `src/node/vfs/ownerfs/bind_mount.rs`; runc adapter is unchanged.

Production Node uses existing mTLS/current-session `PollRootCommandBatch`. The wire adapter validates requested/start/next revisions, event range/order/unique IDs and the nonzero Home tuple; unsupported command types fail closed. Distinct commands may share a transaction revision. Empty and filtered batches preserve the full Meta cursor. RootCommand projection is not a full RootGrant. Exact matching under RootManager's grant lock invalidates local admission and cached peer sessions; wrong well-formed Home/session/epoch/generation commands leave authority intact. Node stops after a matching refusal and uses its existing owned worker join, normal bind detach and FUSE teardown. No successful ACK is emitted. Host ON stops on control errors or recovery-required replies; OFF is unchanged.

The production Meta has no command-issuance entry. `tests/support/root_command_meta.rs` is an explicit Cargo example, starts the real local-file Store and Meta RPC services, validates the current live Home/self-grant and commits test commands with root/grant revision, root epoch, current Node session and request/command absence conditions. It never writes live WAL bytes. It is not a runtime dependency or part of ordinary trial packages.

Reproduce in Linux, with the pinned toolchain and existing dependency cache:

```sh
cargo test --locked --offline --example afs-root-command-meta
cargo build --locked --offline --release --example afs-root-command-meta
# Direct launcher: --config PATH --trigger PATH --receipt PATH --workspace workspace
# Existing processctl takes only --config; test flow can instead supply:
# AFS_TEST_ROOT_COMMAND_TRIGGER, AFS_TEST_ROOT_COMMAND_RECEIPT,
# AFS_TEST_ROOT_COMMAND_WORKSPACE (explicit CLI values take precedence).
python3 -B -m unittest discover -s development/acceptance -p test_workspace_bind_root_command.py -v
```

The maintained [Linux driver](acceptance/workspace-bind-root-command-linux.py) prints its required inputs with `--help`. Before runtime it freezes dependencies, mounts, capacity, source map and all three ELF identities. It uses the fixed f03 package only as a verified carrier for unchanged TLS/config/process-control tools, overlays separately identified new product Meta/Node, bootstraps one existing Home with host OFF, then switches to the test issuer and host ON. Wrong generation then exact matching commands must both commit and match actual production logs. Real Node wait1 must have the command-refusal cause; test Meta and bootstrap incarnations require wait0, owned mounts/PIDs must disappear, full4KiB/dev/inode/permissions and protected inventory remain intact. Failures retain evidence and normally close only the owned fixture. Do not use a receipt's existence or a condition-failed transaction as issuance proof.

[Current limited result and exact identities](evidence/20261008-workspace-bind-root-command/README.md). Existing f03/7e6 standards/performance/recovery retain their versions; the new product source does not inherit complete POSIX or ON acceptance. Production issuer, durable cursor/ACK/recovery, active-runc revoke and immediate FD revocation remain later reliability work. Known append/offset, mixed lock/watch failures remain recorded; no kernel/vendor workaround. Next independent small item: reproduce a default-OFF trial package for the new candidate and verify installed OwnerFs/DFS normal local-file Meta recovery; do not repeat unaffected historical standards or expand the performance matrix.
