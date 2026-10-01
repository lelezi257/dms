# Environment observations

Preparation only: ENV remains PREPARING; no formal case is promoted.

[Linux script tests](inventory-tests.log): nine PASS, including optional command timeouts, missing fields, cgroup ancestors and process identity. Inventory does not read process cmdline or environ. Inputs match [the portable inventory](../../../acceptance/inventory.py) and [its tests](../../../acceptance/test_inventory.py).

Fresh [A](inventory-a.json), [B](inventory-b.json) and [ctl](inventory-ctl.json) observations bind guest kernel `6.8.0-142-generic`, two CPUs and actual process/executable identities. Root-owned A/B process probes initially failed without sudo; [A](permission-original-a.stderr) and [B](permission-original-b.stderr) failures remain. Recollection uses sudo. [Host VM resources](host-vm-resources.json) records C stopped and the independent build VM running; this is not a fresh four-guest lock or performance isolation.

A data ext4 has about 1.1 GiB free, 97% used; root ext4 has about 17 GiB free. Its df command retains readable capacity rows but UNKNOWN overall status because two historical FUSE mounts are disconnected. Those fixtures and data are preserved. Guest block-cache observations and NTP service state do not prove host storage guarantees or actual clock error.

The current preparation audit in the research workspace is `evidence/afs-delivery/environment-preparation-v63/README.md`. Its independently scoped historical ext4 references report pjdfstest 236 files / 8,848 checks / 28 upstream TODO / no unexpected failure or skip, and LTP 657 commands / 605 PASS / 50 TCONF / 2 TBROK / no FAIL or timeout. TCONF/TBROK are not PASS. Those results do not qualify product matrices. Full FSx/random matrices, equivalent MooseFS durable-fsync baseline, three-node 3FS baseline, resource isolation and a frozen environment remain unresolved.

Inventory SHA-256: `b14a1adfa0f03805e07df9030e9f62a45588f78ff9c0ab8153a6bf72a17c54b6`. The three `input-sha-*.txt` files bind actual collector copies.
