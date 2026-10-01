# RDMA admission lifetime slice

Scope: a prerequisite for RDMA-03 resource lifetime, not formal acceptance.
Original source is product main `567ef0fa0350fe80d1d7c94c3ed57122889b3de1`.

The server limit counts registry entries. Close/TTL removes entries while
blocking workers can retain an endpoint Arc. Concurrent negotiations also
check capacity before native allocation without reserving a slot. Admission
must instead follow live endpoint ownership, including unpublished allocations.

First reproduce admission past 64 with actual tiny RXE endpoints retained
after registry close. Fix only existing rpc/control.rs using a pre-allocation
reservation held by the endpoint wrapper. No new wire, modules, dependency,
native provider change, file semantics or acceptance threshold.

Local feedback: original regression, reservation/drop/concurrency regressions,
control tests and affected compilation. Shared endpoint representation requires
early consumer coverage: Owner/DFS data+peer, authorization and actual RXE
file/replica/read integrations. At coherent batch closure run the full Linux
source gate. Full POSIX/performance/8GiB/soak remain scheduled later. Reuse only
unchanged evidence under its original input identity.

64 is a per-registry bound on owned endpoints and pending allocations; this
does not prove cancellation after a posted WQE or exceptional provider native
teardown reclamation. DFS Auto factory selecting gRPC remains a separate gap.

Local closure: 55 distinct Linux regressions passed (control9, original real
RXE1, data22, peer14, Owner contracts9). Ignored library/Owner cases were
not relabeled; explicitly selected native integrations separately pass DFS
replica+read and production Owner4MiB+17. Five existing lifecycle tests pass;
the cancellation fixture uses a delayed fake RPC and does not post data DMA.
143 compiler inputs match the edited host tree. Full gate now runs once at
this batch closure on unchanged frozen r1 inputs.

Build/test only on afs-build Linux/ext4. Idle C is temporarily stopped; restore
ctl/A/B/C and stop build after qualification. Preserve historical fixtures.
AGENTS and docs/handoff.md are unchanged. Formal cases remain NOT_RUN.
