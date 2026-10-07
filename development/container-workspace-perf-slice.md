# Container Workspace Performance Diagnostic Slice

This slice records a small paired diagnostic for the container-mounted OwnerFs
workspace path from Issue42/PR43. It is not a G2.12 function pass, G2.13
performance pass, or production native-workspace qualification.

## Scope

- Candidate package, `afs-meta`, `afs-node`, official `runc`, rootfs inputs,
  `/io`, and `/benchmark` binaries are identity-bound before startup.
- Two sequential cohorts run from the same package and ELF identities:
  default `OFF` (`experimental_native_workspace` false and no
  `[native_workspace]` table) and experimental `ON`.
- Each cohort compares the experiment with a separate ordinary ext4 OCI
  reference container on the same `/opt` ext4 volume.
- `OFF` experiment payloads run through the normal FUSE workspace inside an
  ordinary OCI container.
- `ON` experiment payloads run only through the current Node native controller
  `exec` path. The driver parses the newest controller command artifact by
  exact `runc` argv and never treats the socket response itself as payload
  output.

## Workloads

- Sequential data case: C1, 64 MiB, 1 MiB blocks, byte pattern `90`.
  The timed payload performs `seq-write` with `fsync`, then `seq-read` with
  close. Cache residency is recorded as `unobserved`, never as cold/hot proof.
- Metadata case: `/benchmark` creates, stats, reads, readdir-scans, renames,
  and unlinks 1000 4 KiB files using absolute paths.
- Each cohort runs one warmup and five measurement rounds. Round order
  alternates experiment/reference, then reference/experiment.

## Evidence Boundaries

- The driver captures raw command argv, elapsed time, exit status, stdout and
  stderr for processctl, runc, controller-client, metrics, and payload runs.
- It records runc state, mountinfo, final `/workspace` identity, source object,
  live ELF SHA256, payload SHA256, host load/memory/disk snapshots, and Node
  `/metrics` before and after each round.
- Current Node metrics do not expose exact FUSE request counts for this path.
  Evidence therefore records `fuse_request_counts = NOT_OBSERVED` rather than
  a zero value.
- Any payload failure, shape mismatch, incomplete cleanup, or missing
  experiment/reference pair keeps the result out of completed performance data.
  Existing semantic failures for locks, append offset, and watch propagation
  still block native workspace acceptance.

## Subsequent instrumentation

[First-party callback instrumentation](evidence/20261007-ownerfs-workspace-callback-source/README.md) is source-verified on a later version. New driver scrapes can separately record fuse_callback_counts; the original6d samples and NOT_OBSERVED wire counts remain unchanged. Callback counts cover the implemented first-party entries only. Actual same-Node FUSE-positive/native-bypass witness is still pending; it will not constitute a configuration OFF/ON timing or cache-residency claim.
