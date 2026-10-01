# Scoped independent review

Static review covered Owner transport session authority, file rights, descriptor
bounds, required-mode selection and endpoint ownership. It did not execute tests.

Server findings repaired before r1 validation: authority calls that may block on
the runtime run in blocking workers; length and negotiated capacity are checked
before allocation; cancelled or failed work poisons the session while retaining
its endpoint. Session keys contain exact structured Owner identity and cannot
alias DFS or diagnostic identity. Close validates exact scope without requiring
an unrevoked file grant.

Client findings repaired before r2 validation: prefetch is stored and consumed
only in gRPC mode; admission precedes endpoint allocation and is shared by the
Node factory. A final review found that separate endpoint/permit locals at the
negotiation await released admission before endpoint destruction. The resource
wrapper is now created inside the allocation worker and moves intact through
negotiation, connect/probe and local-buffer workers. Its endpoint field precedes
the permit field, so teardown completes before admission returns. Targeted
static recheck reported no remaining blocker in that path.

Later r3/r4/r5 changes only narrow imports, remove redundant Copy clones,
collapse equivalent prefetch conditions and remove a redundant test default
initializer. The final source gate and RXE fixture use r5 inputs. Earlier r2
integration and non-RDMA registry results retain their own identities.

This is a scoped review, not a complete release security or resource audit.
Posted-DMA cancellation, exceptional provider teardown, loaded-provider identity
and production cross-VM Node/FUSE Owner transport remain unqualified.
