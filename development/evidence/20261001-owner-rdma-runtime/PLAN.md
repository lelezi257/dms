# Cross-VM production OwnerFs RDMA

Status: STAGE_INTEGRATION_PASS with reused frozen v71-r5 source gate; development
integration only. Formal cases remain NOT_RUN.

Reuse frozen v71-r5 Rust source gate and Linux-built binaries. No product code
change is planned. Linux strips copies for deployment and records both hashes.
The build VM is stopped before runtime checks; ctl/A/B/C are restored.

Use independent A/B runtime paths, memory Meta, mTLS, required RDMA/rxe0 and
OwnerFs-only mounts. Preserve all existing service/data paths. Create a workspace
on A and prove its Home via management REST before B writes. Exercise a 4 MiB+17
file, explicit sync, close, B restart to clear client/page caches, full read/EOF
and a small overwrite with another sync/reopen. Bind content to physical A data,
actual server READ/WRITE completion logs and identified Node processes.

Validate the new Linux probe with selected script tests and compilation. Run
real deployments and controlled B restart/stop, record exact source/binary,
config, PID/start ticks, boot and mounts. No full Rust replay for unchanged
inputs. Preserve failures; if a product defect appears, expand its affected
regressions before proceeding. This does not promote formal cases or ENV lock.

Exclude posted-DMA cancellation, provider teardown failure, performance targets
and long stability from this short proof. Handoff remains unchanged.

The two cold reads and patch pass over distinct Linux VM processes. Actual A
completions total READ 4,198,417 and WRITE 8,388,642 bytes. Idle and post-exit owned
resources are absent; both Nodes and memory Meta stop with identified exit 0.
Seven probe checks and seven evidence rejection checks pass. Original REST proxy
502 and auditor label/path failures remain preserved. Client RDMA read/write
timing bypasses the existing timer: record this observability gap separately,
without changing product binaries in this deployment batch.
