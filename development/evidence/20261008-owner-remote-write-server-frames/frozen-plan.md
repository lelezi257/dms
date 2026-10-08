owner-remote-write-server-frames-20261008-r1

Scope:
- Change only the afs-node TCP gRPC server builder used by production Node startup.
- Enable tonic Server::max_frame_size(256 KiB) only when the runtime profile is OwnerFs-only (ownerfs=true, dfs=false).
- Leave DFS-only, combined OwnerFs+DFS, disabled profiles, UDS, Meta, TLS, windows and timeouts unchanged.

Coverage plan:
- Add a narrow production helper in src/node.rs so runtime and tests use the same Owner-only predicate.
- Add a real HTTP/2 TCP handshake regression that reads the server SETTINGS_MAX_FRAME_SIZE value from the wire.
- First verify the existing helper/test shape fails for Owner-only at the default 16 KiB, then apply the runtime change and rerun the same targeted test on Linux.

Validation plan:
- Linux only: rustfmt check for src/node.rs, targeted node.rs test for the HTTP/2 SETTINGS policy, and affected ownerfs_peer_contract mTLS test if compile/runtime remains available.
- No release build, package build, runtime acceptance service, or performance run in this slice.
