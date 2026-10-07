# Workspace probe test boundary

2026-10-07 independent remediation, directly on main. G1 historical8/8 stays
closed; existing candidate ELF/package/results are immutable, not inherited by
the changed candidate. The prepared DFS R3 instance has no started services or
data run; resume that frozen slice after this item.

Current dependency: Cargo discovers src/bin/afs-workspace-probe.rs as a product
binary. The experimental runc adapter hardcodes its rootfs path and idle/identity
arguments for startup and every exec. OwnerFs bind_mount.rs and the independent
host bind worker do not call it. Final-view identity observation is necessary
for the experimental adapter; the particular executable is a test implementation
of that observation contract, not a mandatory product component.

1. Move the unchanged helper to tests/support/workspace_probe.rs and register an
   explicitly built Cargo example. Default product builds (including --bins)
   must not build it. The trial packager continues its explicit Meta/Node list.
2. Require administrator/test-supplied idle_command and identity_command argument
   arrays for experimental container mode. No default test name/path. Validate
   nonempty bounded arguments and absolute nontraversing executable paths, and
   verify trusted rootfs regular executable files for both commands. Old ON
   configs lacking commands fail explicitly; OFF/host bind needs no helper.
3. Keep actual OCI namespace/root/workspace dev+inode, unique mount ID, nosuid/
   nodev, process incarnation, Home/epoch, permission, cleanup and error checks.
   The adapter still parses and validates the observer's JSON before startup
   success and every workload exec. Do not hide this runtime protocol obligation.
4. Update maintained source gates, test rootfs preparation, generated acceptance
   configs, examples and configuration docs; preserve historical evidence paths.
   Build the helper explicitly into release/examples and copy only into test
   rootfs inputs. Add only command validation/spec/error coverage needed here.
5. Linux affected Rust/config tests, fmt/Clippy/build checks; verify Cargo target
   discovery and fresh default-build artifacts, explicit helper build/use,
   actual experimental adapter startup/identity validation/normal closure. Admit
   environment and exact binaries before starting. Real environment blockage:
   save evidence, stop affected lane, ask; no environment repair loop.

Publish source/tools, results and compact evidence index with Lore on main,
ordinary push, no PR/MR or individual human review. Naming/build success alone
does not close full bind or container qualification.

## Scoped progress

11 Linux source gates PASS, including 11 ordinary and 8 privileged native tests,
configuration contracts, separate OwnerFs/DFS checks, strict all-target Clippy,
release product build, explicit example build and helper TERM exit0. Four Linux
rootfs linkage tests PASS; static ARM64 ELF admission requires a valid program
table without interpreter, and missing libraries/errors still reject. Initial
fmt failure is preserved. No third-party changes. Default-build artifact proof,
new ordinary package and real Node/container runtime are still PENDING at this
source commit; final results will be added without overwriting historical proof.

## Final bounded result

Default cargo build and --bins both select only Meta/Node; explicit example
build/use PASS and the ordinary package excludes it. New source7e6/default
ELFs executed one --node-shutdown-only slice on the existing official-runc VM:
startup/identity/read/normal active shutdown PASS, independently confirmed
actual wait0/PID+mount/control closure, frozen inputs and protected objects.
Scoped result/commands/failed-format record and exact maps live in the
[compact evidence index](evidence/20261007-workspace-probe-boundary/README.md).
This independent remediation is complete; G1/G2 broad exits are unchanged.
Return to the prepared DFS R3 one-writer/two-reader small slice; no old result
is automatically upgraded to the changed source or package.
