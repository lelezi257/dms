# DFS Auto transport selection

Original frozen main: e8e5e8e67189dd63b4e0b1986f411778a4f32391.
Scope: RDMA-02/04 automatic preference and typed pre-dispatch fallback, not formal acceptance.

The DFS factories currently choose gRPC for Auto even when an RDMA pool exists;
Node startup creates the pool only for required RDMA. First graft a variant of
the existing actual RXE replica/read integration onto original Linux source,
using Auto factories, and require real RDMA bytes with zero gRPC file payload.
This identifies a behavior failure before implementation.

The accepted safety boundary remains unchanged: an explicit unavailable transport
may select gRPC before data dispatch. Capacity, authentication, malformed protocol,
checksum and unknown completion failures cannot authorize write replay.
Negotiation returns must distinguish capability absence from invalid successful
session identity. Read fallback must not restart a batch after some windows ran.

Validation: selected factory/pool/node startup regressions and necessary compile;
then actual RXE Auto/required/gRPC replica/read, typed absence and fail-closed
integrations. A coherent code batch ends with one final full Linux source gate.
Reuse unchanged tests only with exact input identity. No full POSIX, 8 GiB,
performance or long soak here. AGENTS and handoff stay unchanged.

Build/testing uses afs-build Linux/ext4. If idle C is stopped to free host memory,
record it, restore C/RXE and stop the build VM before closing the slice. Preserve
old runtimes and source evidence. Formal cases remain NOT_RUN/PREPARING.

Original failure captured: after restoring the unchanged RXE environment, the
Auto factory replica/read fixture fails with RDMA replica bytes=0 instead of
8,388,608. Before RXE restore the run failed at local pool creation; that is
retained as an environment failure, not the original behavior regression.
Original source has no implementation edits; the graft only adds a test.

Conservative read fallback: permit whole-batch gRPC selection only before any
RDMA data window command. Once a window starts, later failures propagate rather
than repeat completed windows. This avoids a new mixed-window abstraction.
