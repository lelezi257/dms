# Staged delivery validation

[Acceptance](../docs/acceptance.md) owns targets and topology. The execution manifest expands its cases; neither scripts nor skills may weaken this contract.

## Environment

macOS is for editing, reading and host-side VM orchestration only. Compile, lint, unit/integration tests, probes, filesystem services, faults and benchmarks run in Linux. Acceptance uses the specified dedicated four ARM64 Ubuntu VM topology, ext4 volumes and verified RXE network. Builds use a separate Linux build VM and never compete with a measured lane.

Record exact source and binary identity, environment lock, mount/backend/replica/transport configuration and evidence. Sharing source with a VM is allowed; storing tested data on a macOS share is not. Preserve old experiment data; do not stop unrelated workloads. Before performance runs account for competing VMs/processes and validate resource isolation.

## Feedback stages

1. Environment: pin images/dependencies, verify volumes/network/TLS/backends/RXE, ext4 reference suites, actual MooseFS/3FS mounts and baselines. Missing prerequisites are BLOCKED, not PASS.
2. Fast development: targeted failing regression, short unit/integration and real-FUSE end-to-end cases. Cover chunk boundaries, streaming, memory limits and errors with small fixtures.
3. Feature milestone: applicable POSIX, consistency, etcd/Redis parity and multi-node cases; faults at the modified durability boundary; formatting, workspace checks, clippy and feature matrix.
4. Performance: use frozen representative short workloads to find RPC/copy/serialization amplification, then full contract workloads. Do not start product performance tuning before valid comparison baselines.
5. Release: all mandatory case matrices, 8 GiB files, full FSx seeds, long stability/fault/restart runs, paired performance repetitions and clean/offline installation. Earlier short checks do not replace full gates.

## Trustworthy results

Check content, EOF, attributes, errno, successful durability watermarks and recovery. Verify faults occurred and RDMA file bytes actually used verbs. A TCP fallback is not an RDMA PASS. Track every discovered suite test, including skips/TODO/unfinished items; no post-failure exclusions.

Capture minimal failure sequences and add regression cases when fixing defects. Use PASS/FAIL/BLOCKED/INCONCLUSIVE; NOT_RUN is a preparation status and cannot become release PASS. Publish raw valid paired benchmark runs, not fastest samples. No implicit tolerance or relaxed replica/durability configuration.

An acceptance skill references the same environment lock, case manifest and runner. It must reject missing evidence, stale binaries and altered contracts.
