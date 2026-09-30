# v11 full upstream pjdfstest: memory Meta

Identified live candidate: Meta SHA `8d4af9c9424ee4e96cd08cba892c55d78547fc7f9149604cf680d96742066702`, Node SHA `bd25c7e3b3f3e0d4eca4ed26099d3410691d433097a53ef65bceb1b02eec5364`.
Runtime: isolated A guest ext4 memory Meta17800/17801 and Node17802/17803, independent DFS/Owner mounts. Pinned suite revision and live process/mount identity are in each `identity.json`.

- DFS: PASS,236files/8819checks,0unexpected,28upstreamTODO,0incomplete,1418seconds.
- OwnerFs: FAIL,236files/8819checks,2unexpected,28upstreamTODO,0incomplete,154seconds.
- Owner failures: ftruncate/00.t check24 wrongly rechecks mode0 path permissions on writable fd; unlink/14.t check4 fails getattr of unlinked still-open inode. Fix/regressions await new identified candidate replay.

Raw TAP, command, discovery, accounting and proof files are immutable. No tests excluded after failure. This single-node memory-backed core functional result is not a release matrix or persistent recovery claim.
