# P2 memory FUSE lane evidence

Run id: `20260930T220714Z`.
Mode: `keep`.
State after script: running.

This is a bounded, memory-backend, single-node functional lane on `afs-accept-a`.
It uses isolated ports `17980/17981/17982/17983` and isolated guest runtime `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v48`.
It does not use or modify existing ctl `17500/17501` or NodeA `17400/17401` services.

Candidate binaries staged from host `/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/.local/v48-binaries/afs-meta` and `/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/.local/v48-binaries/afs-node`:

```text
f96b6a06047da02bf186ea8d89457ee74c454d6c437915f5c32a20a9b909f312  /Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/.local/v48-binaries/afs-meta
f84b641409d1f60de5e811122f0e7e2694d31b581823d75b3406996a1b875f39  /Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/.local/v48-binaries/afs-node
```

Meta backend: `memory`. This lane intentionally does not prove restart durability.

Validation performed when present in logs:

- Meta health ready.
- Node health ready.
- DFS and OwnerFs FUSE mounts present.
- Actual process `/proc/<pid>/exe` SHA captured for Meta and Node without sudo-wrapper PID capture.
- OwnerFs create/write/fsync/close/reopen short proof.
- DFS create/write/fsync/close/reopen short proof.
- OwnerFs root Home REST query against memory Meta.

This is not a full acceptance pass and does not run pjdfstest/performance suites.
