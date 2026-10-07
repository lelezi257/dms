# Workspace bind: confirmed data process recovery

**Fact, 2026-10-08:** the fixed f03/7bfc runtime recovered its pre-existing host workspace after the previous registration-epoch error closure. **r2 PASS / 44 checks**, one Meta/Node start-stop cycle. G2.12 remains in progress; G1 historical 8/8, G2 totals and default OFF are unchanged. The first driver preflight was **BLOCKED**, preserved below.

| Item | Version, range and evidence |
| --- | --- |
| Source context | [cf285d71](https://github.com/lelezi257/dms/commit/cf285d71640d6042e972ae4049677e41e076b6a3); acceptance tools/documents only, no Rust/vendor change. [Impact](source-impact.json) |
| Actual product | [f03dc2b3](https://github.com/lelezi257/dms/commit/f03dc2b3679c31daa51caee275fb2087413e949c), package SHA256 `7bfc6b520e551d2281790862c2968e990e376c5c490b2135f583f4da0ccc1972`; Meta `c7447bcfac7e3f8bf605446ade5e11be74f1333709a8f7bb5378b1d6ee7506fd`, Node `3b1f1dce187a6285814c03b9024cdfc5f2dec990a73cba9cc13b3dc9ef402d36` |
| Environment | Existing afs-g2-micro ARM64 Linux, guest ext4 UUID recorded, local-file Meta, gRPC/mTLS; no VM reset, new fixture, reinstallation or payload recreation |
| Prior state | Same `/opt/afs-workspace-bind-epoch-20261008-r2` root, configs, installed prefix, TLS identities, Meta WAL, Home catalog and physical data; exact prior archive hashes checked **before launch**. Old actual Node error exit1 and Meta normal exit0 captured before processctl legitimately retired their stale lifecycle generation. [Previous receipts](r2--results--previous-actual-receipts.json), [checks](r2--results--checks.json) |
| New authority | Fresh Node session `9f2efeb4-66b3-41d6-b2ae-847422a845a8`, epoch **5**, greater than prior original2 and trigger3, persistent Meta ready. [Health](r2--results--recovered-health.json), [incarnations](r2--results--recovered-identity.json) |
| Actual bind | Same physical `state/node/ownerfs/root-776f726b7370616365-e1` → `mount/ownerfs/workspace`; ext4, device/inode **64769/829730**, new mount ID91; not a bind of FUSE itself. Container adapter OFF. [Binding](r2--results--binding-recovered.json) |
| Confirmed data | Existing 4096B `epoch-proof`, full content and EOF exact, SHA256 `c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193`, same file inode829768, uid/gid0, mode0644. Read through recovered workspace without writing. [Before](r2--results--proof-before.json) / [recovered](r2--results--proof-recovered.json) |
| Normal closure | Meta/Node actual wait0 with exact lifecycle/incarnation receipts; children and supervisors gone, bind and FUSE absent, protected inventory unchanged. [Receipts](r2--results--recovered-actual-waits.json) |
| Budget | Final allocated **82,616,320B**, below256MiB; guest free **6,999,556,096B**, above1GiB floor. Existing prefix/config/proof and tool inputs unchanged. [Budget](r2--results--budget-final.json) |

[Measured result](r2--results--result.json), [44 checks](r2--results--checks.json), [commands](r2--results--commands.json), [pre-launch plan](plan.md), [frozen contract](r2--results--contract.json).

## Failure accounting and reproducibility

**r1 BLOCKED:** the new driver omitted the inherited `check` API's required evidence argument at the owned-process preflight. No product service was started, no state was recreated or changed. [Original result](r1--results--result.json), [preflight commands](r1--results--commands.json), [original checks](r1--results--checks.json). Both call sites were corrected; a targeted inherited-API guard was added. [Eight Linux tests](unit-checks.json) pass, covering complete content/EOF, symlink refusal, ownership/inode equality, fresh persistent session, immutable inputs and the API contract. This is a test-tool correction, not an environment repair.

[Driver](../../acceptance/workspace-bind-recovery-linux.py) accepts the **existing** root, previous results and SHA-bound previous index; it does not install a package or generate data. [r1](tool-inputs-r1.json) and [r2](tool-inputs-r2.json) exact maps differ. [Delta](r1-to-maintained.patch) restores the original executed r1 entry/test from maintained files with `patch -R -p1`; all other helpers are shared. No full scripts are copied into Git evidence.

Invocation, inside the existing Linux VM as root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 /var/tmp/afs-workspace-bind-recovery-20261008-r2/tools/workspace-bind-recovery-linux.py \
  --root /opt/afs-workspace-bind-epoch-20261008-r2 \
  --transport /var/tmp/afs-workspace-bind-recovery-20261008-r2 \
  --out /var/tmp/afs-workspace-bind-recovery-20261008-r2/results \
  --inputs /var/tmp/afs-workspace-bind-recovery-20261008-r2/inputs.json \
  --previous-results /var/tmp/afs-workspace-bind-epoch-20261008-r2/results \
  --previous-index /var/tmp/afs-workspace-bind-recovery-20261008-r2/previous-index.json \
  --source-commit f03dc2b3679c31daa51caee275fb2087413e949c \
  --afs-meta-sha256 c7447bcfac7e3f8bf605446ade5e11be74f1333709a8f7bb5378b1d6ee7506fd \
  --afs-node-sha256 3b1f1dce187a6285814c03b9024cdfc5f2dec990a73cba9cc13b3dc9ef402d36 \
  --processctl-sha256 1ea10552180c917da2c598bee9dd8fecee7be26ae9d70dcdcba2e4829150d896 \
  --inputs-sha256 61837e9f0492683f631c2946cfeb0f40668782838e0243415441b82647efa088 \
  --previous-index-sha256 94c646d62652708de4b423fc89f59f4ca3230785673768eabb201cc43129bdfd
```

This exact one-shot invocation now has already been executed; the preserved old state has advanced and must not be silently reset/reused as another equivalent run.

## Archive and scope

Raw archive outside source: `evidence/afs-delivery/workspace-bind-recovery-20261008-r1/guest-evidence.tar.gz`, **24,392B**, SHA256 **bd29780fae6d5d29945ecd7ae0d76a9eebc171e59d1e6139f017e060762f4419**. [Index](raw-evidence-index.json) covers97 files including failures, command streams, logs, state/proof and final actual receipts. Linux actual extraction checked all97 files; both executed tool maps restored/verified18 files. [Restore result](restore-verification.json). Initial read-only archive audit assumed JSON receipt format instead of actual key=value; [failure retained](restore-r1-failure.json), parser corrected without another product run. TLS keys, ELFs, VM images and full tool copies excluded; guest roots retained.

**Scope:** process restart after heartbeat-driven authority-error closure with no outstanding native FD/mmap. The proof was file-fsynced and closed; its parent entry was not separately directory-fsynced. This is **not** power-loss/crash/VM-reboot durability, immediate revocation of live FD/mmap, runtime root-command watch/ACK, complete POSIX, full bind function or performance qualification. The [prior epoch case](../20261008-workspace-bind-epoch/README.md) retains both original FAIL results; this new recovery does not rewrite them or re-run previous standard/performance cases. Remaining necessary bind semantics include mixed-path append/current-offset/lock propagation and authority invalidation; ordinary Moose comparison qualification remains a separate stopped lane.
