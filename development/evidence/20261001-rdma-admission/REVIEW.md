# Static admission review

Independent native lifetime reviewer: zero blocking findings. This is static
code review; the reviewer did not build or execute tests.

The server endpoint Arc owns its native mutex before its semaphore permit in
field order. Independent diagnostic endpoint clones therefore retain admission
after session removal. Reservation precedes spawn_blocking, whose closure owns
the permit through allocation, failure or result release. Atomic semaphore
acquisition prevents concurrent negotiations from overbooking slots.

New members are RDMA feature gated. Default delegates to new(None). Existing
capacity/native/internal error identities are preserved. Operations wording
uses a per-registry bound and excludes provider leak or posted-DMA claims.

Final-review changes: public RdmaSession.endpoint becomes
Arc<RdmaServerEndpoint>, transparently dereferencing its native mutex. Public
ensure_capacity is replaced by private reserve_admission; repository consumers
use negotiation. Default now uses normal 600s TTL and next-ID1 rather than the
derived zero initialization. No wire, disk format, module or dependency change.

Actual cancellation after WQE posting and exceptional native provider
destruction/reclamation remain unqualified; this fix establishes ownership
accounting, not cancellation of physical DMA.
