# OwnerFs workspace bind mount naming and ownership repair

2026-10-07. **Facts:** this is a narrow, independently reviewed ownership repair after the [3cc candidate's actual orderly-recovery slice](../20261007-native-orderly-recovery-runtime/README.md) was finished and preserved. G1 remains historical8/8; G2 remains10 bounded done/2 ordinary performance FAIL/2 bind in progress/13 pending. This is not full bind functional acceptance or performance qualification.

## Source and actual mount boundary

Core is the single [src/node/vfs/ownerfs/bind_mount.rs](../../../src/node/vfs/ownerfs/bind_mount.rs), owning mount, descriptor/namespace/directory/unique-mount identity and normal unmount; no bind_mount/ directory or fixed container target. [runc adapter](../../../src/node/native_workspace.rs) keeps start/exec/stop, rootfs and control; Node keeps lifecycle wiring. WorkspaceBindMount replaces ManagedExport; secondary-clone inspect/detach take a validated component. Adapter selects workspace; the privileged generic test uses project. [Old/new traceable mapping](path-mapping.json) preserves the old source exactly through Git without another full snapshot.

Authorized source is the Home backing directory under node data_dir/ownerfs, root-<workspace-name-hex>-e<epoch>, opened no-follow through the existing Home/root/epoch grant. Target is <OwnerFs FUSE root>/<workspace name>, one first-level component. It is the physical Home directory, not the FUSE directory rebound to itself. **Existing first-level covering mount is visible only inside the controller's private mount namespace**; ordinary host/Node callers still see FUSE. runc subsequently binds that view into /workspace. A standalone host-visible switch and multi-workspace Node lifecycle remain unimplemented; naming does not close this design gap. No direct-I/O/mmap negotiation, third-party source, authorization/cache/permission policy or config parsing change.

## Current source validation (new identity)

Base commit 6e57374e8755e859db6c06b26ed5abf0fe948f80; [157 frozen inputs](compiler-inputs.json), map fabab19aebc1390200399ebe01c9df1b09256dc43fb00924f1a33a646c9990c7. Initial manifest retains its pre-test FROZEN_DRAFT_NOT_VALIDATED state; subsequent [source proof](source-proof.json) is PASS and input hashes are unchanged at entry/exit. Linux ARM64 only, existing warm release target, no environment rebuild. Selected checks: bind ordinary3PASS; adapter ordinary8PASS; config7PASS; actual root bind4PASS; actual rootfs3PASS =25 distinct selected tests. The seven explicit ordinary ignores were all executed by the precise privileged filters, not counted as ordinary PASS. fmt, strict workspace/all-target/all-feature Clippy and release build also PASS; no full POSIX/standard/performance/package or new product runtime qualification. [Commands](physical-bind.command.json), raw *.log/exit and [test ELF identity](test-artifact.json) are retained.

New built Meta SHA e4d37d1af1010183a2f5fba86c7557728502de383b80d0657d9d34d5907fb4da; Node SHA b235fc862454fa97196ed42633d948faf2fef931a7ebdf6b97dff147342e5429. These ELFs were not packaged/deployed; old 3cc runtime PASS is not reassigned to them. Public default-OFF6d trial stays unchanged.

First formatting write failed because Linux shared source is read-only: [stderr](format.stderr). [Resolution receipt](format-receipt.json): Linux rustfmt emitted text, host applied exact emitted source, final Linux fmt check passed. No mount/permission/environment repair; large emitted full-source copies stay outside Git. [Independent final review](review.json) approved the fixed source hashes.

## Configuration and remaining acceptance

Keep experimental_native_workspace, [native_workspace] and existing CLI spelling intact as runc-adapter settings, with explicit OFF in both node examples/templates. No renamed key or silent ignore; existing config contracts verify strict unknown-field rejection, incomplete enablement and explicit OFF override. Feature name is OwnerFs workspace bind mount; container is an adapter scenario.

Full G2.12/13 stays in progress: ordinary-caller visibility/standalone lifecycle, append offset, classic kernel-lock coherence, cross-watch, revoke/drain and restart reconciliation remain unqualified. Existing failures/data and links are preserved; no performance re-run or standard-suite blanket claim. Return to the established bounded E2E queue; independent Owner remote B-Home small read/delete can proceed with its existing6d identity while these functional gaps remain separate.

Only commands/results/versions/checksums/index are stored here; no script snapshot, ELF or archive. SHA256SUMS covers every other packet file.
