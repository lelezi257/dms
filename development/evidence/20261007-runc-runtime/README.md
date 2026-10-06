# Official runc installation and runtime admission

2026-10-07, isolated ARM64 Linux VM `afs-g2-micro`. Human authorization: “在隔离 VM 安装官方 runc”. **PASS for environment admission only**; no AFS code, vendor, candidate ELF or historical acceptance conclusion changed. This resolves the missing-runtime observation in the [earlier source slice](../20261007-native-workspace/README.md), without rewriting its raw BLOCKED record.

| New result | Evidence | Scope |
| --- | --- | --- |
| Official installation PASS | [proof](installation-proof.json), [version](installed-version.stdout), [stat](installed.stat), [release metadata](release.json) | Fixed official [runc v1.5.2](https://github.com/opencontainers/runc/releases/tag/v1.5.2), ARM64 asset installed `/usr/local/sbin/runc`, root:root0755,10,091,904B |
| Authenticity PASS | [official checksums](runc.sha256sum), [installed hash](installed.sha256), [GPG status](signature-imported.stdout), [key import](key-import.stderr) | SHA256 matches official checksum and GitHub asset digest. Detached signature validates fingerprint `C2428CD75720FACDCF76B6EA17DE5ECB75A1100E` from pinned official keyring |
| Real container lifecycle PASS | [commands/exits](runtime-commands.json), [result](runtime-result.json), [OCI spec](admission-spec.json), [initial inputs](rootfs-inputs.json) | create/start/state/identity/exec, UID/GID501, zero effective capabilities, noNewPrivileges, read-only rootfs, ext4 workspace write/read/sync. Six namespaces. TERM→stopped→normal delete; no force/lazy cleanup |
| Postcondition PASS | [live observations](postcondition.json) | Runtime list empty (`null`), no afs-node/meta/probe/runc process or selected FUSE mounts; ext4 available12,362,956,800B. `ps`/`findmnt` exit1 with no matches is expected absence |

Installed SHA256: `d10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0`.

## Commands and retained errors

Downloads use official release URLs in `release.json`; `runc.keyring` comes from `https://raw.githubusercontent.com/opencontainers/runc/v1.5.2/runc.keyring`. Verification uses `gpg --homedir <isolated-keyhome> --import runc.keyring`, then `gpg --homedir <isolated-keyhome> --status-fd 1 --verify runc.arm64.asc runc.arm64`; install the verified asset as root with mode0755 to `/usr/local/sbin/runc`. The installed file was then independently rehashed. These installation steps are an orchestration transcription; actual verification/download outputs, live installation proof and every runtime command/exit are retained.

[Preparation errors](preparation-failures.json) include the first incorrect keyring invocation (raw retained), the static BusyBox ldd assertion, and blank checksum-line parsing. They are tooling/preparation errors, not hidden runtime failures; no environment rebuild or additional dependency install was used to resolve them. Claims are limited to final recorded checks.

## Next boundary

This container used a guest ext4 directory, **not OwnerFs or its managed controller**. The helper belongs to unchanged source25a8061/map8ef8b788; its SHA is in `rootfs-inputs.json`. G2.12 functional and G2.13 performance exits remain open, counts unchanged, default OFF.

[runc's rootfs changes](rootfs-after-runtime.json) show devices/symlinks left under `/dev` after delete. The AFS controller's trusted-tree startup admits only regular files/directories, so the next managed-container fixture must be built cleanly from pinned inputs rather than reusing this mutated rootfs. Restart/rootfs reconciliation remains unqualified; runtime admission does not resolve it.

Next: managed single OwnerFs workspace final-view/exec/normal-stop and detach, then selected required semantic/failure cases before bounded OFF/ON/ext4 paired performance. Keep ordinary baseline FAIL/data and historical G1 8/8 unchanged.

Only text metadata, checksums, signatures and command outputs are versioned. Runtime ELF, helper, keyhome and rootfs remain outside Git. `SHA256SUMS` covers this packet except itself.
