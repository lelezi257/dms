# Deferred etcd memory topic

**Decision, authorized by the user:** native etcd may use a **2GiB (2,147,483,648-byte)** cgroup budget; memory-growth attribution/permanent repair (historical D17) remains a separate topic. [G3.12](trial-release-goals.md) places etcd after the usable local-file/core-performance version; Redis is last G3.13. This is a scope adjustment, not a claim that growth is fixed.

The exception concerns native etcd sustained-growth/idle-baseline attribution in REL-14/OPS-05. It does not waive AFS correctness, controlled resource use or actual OOM/unsafe success/corruption/permission failures. Meta and Node budgets remain unchanged. Any future etcd run binds actual limit, process identity and observations; old1GiB results keep their original scope. Long soak and backend fault/parity tests are G3 tasks, not G1/G2 preconditions.

**Historical facts:** v122 completed original default fcntl14 in29.877s under1GiB; client-free native reached926,220,288B withoutOOM. Later2GiB health/recovery and exact pending-operation ACK replay slices were independently checked, but full backend cases remain unqualified. MVCC compaction/heap samples do not prove an object-retention graph or a permanent repair. Research-only red tests were kept outside formal product source.

**Archive boundary:** original raw packets remain in research `evidence/afs-delivery/`, including native-raft-compaction-v147, heap-debug-summary-v152, health-installed-v156-runtime and meta-reload-v215-runtime; they are not included by standalone Git clone. See [portable current scope/evidence](current-checkpoint.md). Next work remains Owner/local-file/core performance; revisit the etcd topic at G3.12 rather than continuing memory tuning now.
