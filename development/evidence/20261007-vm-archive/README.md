# Build VM historical archive

2026-10-07. User requested a space breakdown and host-side archival. **Archival completed; VM originals retained and no bytes freed.** This is not an acceptance run, a build, or an environment repair. G1 8/8, published 6d trial and existing PASS/FAIL conclusions are unchanged.

`/home/lzc.guest/afs-build` has accumulated old test ELF/package copies, build trees and preserved caches. The archived evidence/artifacts/logs roots occupy about37.65GiB by combined `du`; several historical test ELFs are300–360MiB each and two preserved cache archives are about1.2GiB each. Baselines separately occupy17.2GiB, old target/debug8.4GiB, current staging debug2.54GiB; they were not archived or removed. These are point-in-time inventory values, not a claim that all data is disposable.

[Archive receipt](archive-receipt.json): source roots streamed from GNU tar in Linux to host zstd,10,194,701,840B compressed, SHA2564e90f6afb097e4dea48ec744fafa50e72f06835cf9fc3f288a8ac7119b2311ee. Linux archive readback checked11,852 entries/10,661 files: every file SHA/size, path/type/mode/UID/GID/mtime/link target matches; before/after source manifests identical. ACLs/xattrs encoded by tar but not independently compared. No full filesystem restore or old runtime identity replay is claimed.

The complete archive, manifests, verification tools and restore procedure live outside Git at `evidence/afs-delivery/vm-archive-20261007-r1/` in the research workspace. This Git packet contains only the small receipt, [summary](inventory-summary.json) and [raw disk inventory](guest-space-inventory.txt); no ELF, full script snapshot or large compressed archive. Historical raw failures and existing evidence links are retained.

**Pending user choice:** whether to remove only the already archived evidence/artifacts/logs VM roots. Current release cache, source, baseline and toolchains remain protected. Root `du` is a reclamation estimate; external hardlinks/open descriptors can retain blocks, so actual freed space must be measured by `df` after any authorized removal. Build VM is still full; orderly-recovery final release checks/new package/4KiB runtime remain blocked.

Retention decision: archive historical large assets on the host and reference by SHA; keep results/commands/version/hash/failure indexes. Keep one current build cache and avoid per-run debug/root Cargo toolchain duplication. This record does not implement cache cleanup or change performance requirements.
