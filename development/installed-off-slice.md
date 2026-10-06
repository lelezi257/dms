# Current default-OFF installed trial regression

This independent G2.08 branch qualifies main25a8061 release binaries after default-OFF native source integration. It does not reopen G1 or complete G2.27/G2.12/G2.13. [Results and identities](evidence/20261007-installed-off/README.md).

Use the existing ARM64 Linux compiler-free VM, fresh isolated `/var/tmp` paths, local-file Meta/gRPC/R1 and ordinary OwnerFs/DFS mounts. Freeze package/source/ELF/tools/dependencies/ports/mount paths and at least1GiB free ext4 capacity before start. No environment repair or dependency installation is part of the driver.

Installed selfcheck writes/verifies64MiB per backend and basic operations; sync parent directories, restart only Meta, preserve Node/mount ID, then re-read complete content/EOF. Cleanup requires managed exit0, before/after process absence and exact mounts removed. Errors retain commands/output and cannot count as success. Seven targeted Linux tests protect false-PASS boundaries.

On Linux, with the separately transferred archive (not in Git):

```sh
sudo python3 development/acceptance/installed-smoke-linux.py \
  --package /var/tmp/afs-0.1.0-g2-off-25a8061-linux-aarch64.tar.gz \
  --root /var/tmp/afs-off-delivery-new \
  --out /var/tmp/afs-off-delivery-new-results \
  --package-sha256 77cbae8914715cd08d6ba3771e0219111f18d5cc0475f42661e4913873bfb18b \
  --source-commit 25a8061a82b7a8629c9a06486d712d32507f2ee7 \
  --afs-meta-sha256 08aee9fec2e7e12ce03912fa85e8bded28b2162de78c2c9c36703a9a1a6e58b8 \
  --afs-node-sha256 d6e7cd11a610e7f0839d2a3d2337cd92678432e15fcffb6fd2fa6a00860058b8
```

Existing roots/output dirs refused; ports22400/22401/22500/22501 must be free. Git not used; compilers must be absent. Standard suites can use their separate qualified Linux environment. Basic selfcheck is not complete POSIX; orderly central restart is not crash recovery.

First performance priority remains container-mounted workspace access. Missing runc blocks only that runtime lane; this check triggers no ordinary benchmark/tuning. Package excludes experimental helper/control and does not qualify ON.
