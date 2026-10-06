# Owner remote read/delete bounded environment admission

2026-10-07. Read-only ctl/A/B inventory, not product acceptance. No VM/service start, stop, configuration, fixture creation, data read, timing, cleanup or expansion. Commands ran once on three already-Running guests.

**Fact: BLOCKED for a new64MiB historical A-Home/A-Moose-chunk paired fixture under the preserved4GiB reserve.** A data available4,322,222,080B, only27,254,784B above reserve.64MiB +100x4KiB requires at least4,362,485,760B; shortage40,263,680B, before candidate deployment/state/log. The affected lane stopped. A actually has **4.0254 GiB available**, not a full disk. The stop is under the preserved old-helper reserve rule.

| Guest | Actual IPv4 | Kernel | Data availableB | Fixed4Moose ELF | Old Moose mount/process | Other cohorts |
|---|---|---|---:|---|---|---|
| ctl |192.168.109.11|6.8.0-142-generic|4,481,642,496|4/4match|absent|2AFS Meta/Redis; old ext4LTP loopmounts|
| A |192.168.109.12|6.8.0-142-generic|4,322,222,080|4/4match|absent|7AFS Meta/FDB; old Owner/DFS FUSEmounts|
| B |192.168.109.13|6.8.0-142-generic|18,827,644,928|4/4match|absent|none in recorded relevant process/FUSE inventory|

All three data targets are guest ext4; base commands and four ELF dynamic libraries are available. Recorded20940–20943 ports are unoccupied. The prefix is `/opt/afs-moose-round3-v85`, stock4.59.2/ac106b2; fixed hashes bind the prior build. Full226prefixmanifest, supported class tools/config, current6d candidate deployment, futureports/TLS and bidirectionalTCP were not admitted.

The v89 mount-path file stat is absent on A/B because the exactMoose mount is unmounted. This does **not** establish that retained backend data is lost; no full32MiB read or backend mutation was performed. Old process inventories and mounts remain untouched.

B and ctl meet the lower-bound data-space check. This is potential independent space, not permission or proof for a swapped topology or a fully isolated performance environment. A/ctl othercohorts prevent a wholeVM isolation claim.

Raw controller argv/stdout/stderr and executed guest query input are stored alongside `summary.json`; the latter includes raw guest observations and derived decisions. `SHA256SUMS` binds packet files.

Reserve provenance: `experiments/afs-acceptance/probes/round3-moose-read.py:34` sets `RESERVE=4*1024**3`; `:187` generates `HDD_LEAVE_SPACE_DEFAULT=4GiB`; `:202–205` checks available space. This is the historical helper/comparator policy. The afs-acceptance skill does not specify a numeric reserve; this inspection does not establish a newly frozen 64MiB contract with that number.

Actual Lima resources: ctl 2 CPU / 4GiB RAM; A and B each 2 CPU / 6GiB RAM. ctl currently hosts two old AFS Meta processes plus Redis; A hosts seven old Meta processes plus FDB and two old FUSE mounts; B has no matching relevant process/FUSE cohort. Those are retained old duties, not new-test services.

Inference only: new Home/sole Moose chunk on B, with A client-only and small metadata/state, is a plausible placement under the observed capacities while retaining the 64MiB workload and correctness predicates. It is **not yet admitted**. The old helper hard-codes A chunkserver (`:179–188`, `:263`), A writer (`:503`) and A topology identity (`:454`); the old policy driver also requires A for fixture creation. A role swap requires a distinct new fixture identity/config/topology proof, capacity for candidate/state/log/client cache, and fresh candidate/TCP/mount ownership admission. Historical PASS cannot be transferred to this topology. No such action was taken.
