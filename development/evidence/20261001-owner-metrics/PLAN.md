# OwnerFs transport observability

Accepted scope: OPS/RDMA first-stage observability, preserving existing OwnerFs
Home authority and logical I/O completion. No RPC schema, replica, persistence,
module or dependency change. Memory-backed Linux functional development.

The prior cross-VM flow proves actual file DMA but the Owner RDMA client returns
before its existing method timer, and no Owner payload counter can distinguish
successful gRPC/RDMA bytes. Fix within existing rpc.rs/peer.rs/data.rs.

1. Capture a failing metrics regression against the original frozen v72 product.
2. Count one logical remote client operation, including negotiated RDMA and Auto
   fallback, in the existing timing metric. Cache-only prefetch is not an RPC.
3. Add bounded side/direction/plane payload counters for verified successful
   logical read/write bytes. Never count requested length, handshake messages,
   failed/malformed/corrupt completions or cache-only bytes. EOF adds zero.
4. Run the original failure, affected Owner/RPC module tests and necessary compile.
   Validate both endpoint counters and actual RXE client/server byte agreement.
5. At batch closure run the complete Linux source gate and feature checks for
   the frozen candidate. Preserve earlier runtime evidence under its identity.

The local feedback loop does not replay the full gate after every edit. Native
verbs/provider lifetime implementation is unchanged, so previous tests remain
qualified only for those exact inputs. Fresh metric integration does not qualify
full cancellation, failure, performance, POSIX, 8 GiB or soak matrices.

Root owns Linux build/runtime orchestration and publication; a bounded executor
owns only the four existing implementation/test files. C is idle and temporarily
stopped while the separate Linux build VM runs. Restore ctl/A/B/C before runtime
checks. Preserve previous services and data; do not refresh handoff.
