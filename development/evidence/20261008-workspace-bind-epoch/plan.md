# G2.12 narrow real session-change closure

Decision: reuse the fixed f03 / package7bfc candidate on existing afs-g2-micro. Product inputs and protected services remain unchanged; only add a maintained Linux acceptance driver and targeted guards. G1 stays closed, full bind and performance conclusions unchanged.

Preflight once: dependencies, exact package/tool SHA, ELF/ldd, Linux ARM64 root, ext4/FUSE, six ports, protected processes/mounts, 256MiB case budget and 1GiB filesystem floor. Stop the affected lane on an actual environment blocker; retain original failures.

Case: isolated local-file Meta; bootstrap one workspace with bind OFF; normal close, then actual host bind ON. Write/sync/close 4096B and prove physical ext4 source equals FUSE/workspace target. Start one second ordinary OwnerFs Node with same already-trusted node id/certificate, fresh automatically generated session, separate data/mount/ports and bind OFF. Public registration must expose a greater epoch through health. Capture that replacement's identity, then stop it normally to avoid competing renewals. Observe the original's next heartbeat without signalling it: authority error/nonzero actual wait, normal bind and FUSE removal within 35s, no forced124/SIGKILL; physical proof unchanged. Stop Meta normally and verify protected inventory/budget.

The original heartbeat may advance epoch again; do not require the replacement to remain current. Does not prove immediate native-FD denial, RootManager runtime watch/revocation integration, durable revoke/ACK, full POSIX or performance. Current production cached-root invalidation/watch wiring is a separately recorded gap; do not implement broad watch protocols in this slice.

Checks before runtime: replacement configuration cannot share source/mount/UDS, explicit bind OFF and same cert/id; verifier rejects unchanged session/epoch, wrong-node health, success124/signal or mismatched wait incarnation. Run only these relevant Python checks on Linux, then one real case. No Rust or third-party edit/build required.
