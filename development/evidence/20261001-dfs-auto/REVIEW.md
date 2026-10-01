# Read-only scope findings

Code evidence: peer factories map Auto directly to gRPC; Node pool preparation
requires data_mode==rdma. Pool acquire currently conflates rdma_supported=false
with malformed negotiation. The server validates authentication/read or replica
authority before transport negotiation; preserve this ordering.

Change only existing Node startup, Peer adapters/pool and shared negotiation
helper. Keep public constructors/traits, protocol layout, features and dependencies.
Use typed predispatch absence only. Do not catch whole write/read adapter errors.
Read batches may have multiple windows: reject later errors after data dispatch
instead of whole-batch restart. Use the operation remaining deadline for fallback.

Existing diagnostic connect_data_client catch-all fallback is separate pending
work; do not copy it into DFS. Posted-DMA cancellation, provider exceptional
reclamation, peak resource and complete fault matrices remain open.

Native original test reuses real production DFS handlers and metrics; no fresh
cross-VM Node deployment is claimed. Static review is not an executed test.

Writer/root checkpoint: reject a whole-adapter error wrapper that identifies
predispatch failure by error message text. A remote data RPC can return the same
code/message after dispatch; that cannot authorize write replay. Carry a private
phase outcome created only by acquire-before-command, or select fallback directly
at acquire inside the existing adapter. No two extra Auto wrapper types are needed.

Final read-only review found zero blocking correctness issues. Cleanup now
precedes false-reply validation, so malformed nonzero sessions remain eligible
for best-effort close. Only acquire-before-command constructs the private
fallback phase. The read loop permits fallback only at the first window and
passes the original remaining deadline to gRPC. Authentication and authority
validation precede unsupported negotiation. The required-negative test initially
returned after write refusal; root extended it to execute read refusal too.
The final Linux related logs prove both. Late-window, postdispatch and deadline
faults remain unexecuted; review does not qualify them.
