# OwnerFs workspace bind host entry

Scoped implementation on main, default OFF. The first entry binds one already-existing Home workspace into the matching first-level OwnerFs FUSE directory in the Node startup mount namespace. It does not require runc. The existing private-namespace container adapter and configuration remain compatible and mutually exclusive with this entry.

Before changing behavior, add configuration regressions for default/override, role/backend, a single safe component and incompatible modes. Add authorized-core regressions for wrong parent/source and a real FUSE mount, plus worker ownership/error propagation coverage. Reuse existing Linux source admission, cache and physical test gates; record frozen source and binary identity. Do not inherit old runtime PASS for a new binary.

The core stays in `src/node/vfs/ownerfs/bind_mount.rs`: authorized physical Home descriptors, exact namespace/directory/mount identity, normal unmount retaining claims on failure. Node owns worker startup/stop/join before FUSE teardown. Failed or partially attached startup keeps the claim alive until normal closure, with the existing process watchdog bounding unresolved closure.

This is an administrator-controlled, fixed-Home experiment. Linux DAC governs native accesses; ordinary native operations do not perform per-operation RootGrant checks. Monitoring a local authority does not establish immediate revocation of existing FDs, mmap, descriptor transfers or secondary clones. Administrators must stop managed users before Node shutdown or root lifecycle changes. Automatic new workspace adoption, multiple roots, crash/restart reconciliation, dynamic revocation/drain and full ON qualification remain pending.

Validation status: scoped implementation and [12 Linux source gates/50 selected checks](evidence/20261007-ownerfs-workspace-host-entry/README.md) passed on frozen157-map9661a313; source-only new services, packaging/deployed Node host acceptance pending. Historical G1 8/8 remains closed. G2.12/G2.13 remain in progress; naming, build or a bounded mount test does not constitute full bind qualification.

Installed fixed-Home host runtime subsequently passed on93169c8: [scope, actual evidence and original failures](evidence/20261007-ownerfs-workspace-host-runtime/README.md). Full ON/current throughput still pending.
