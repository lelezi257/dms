# OwnerFs workspace bind: held-file and mmap drain slice

2026-10-08. G2.12 supplement; historical G1 8/8 stays closed. Test-only change on main18554a47; ordinary deployed f03/7bfc identities remain unchanged.

Existing real-FUSE tests cover cwd EBUSY and authority validation failure after all files close. They do not hold a regular-file FD or mmap across authority-cache invalidation. Add one exact ignored Linux ARM64 real-FUSE test in the existing native_home_fuse_fixture.rs, using AuthorizedWorkspaceBind and existing normal-unmount/session guards. No product or vendor changes.

Pass: physical Home covers the first-level OwnerFs FUSE directory; file FD mount ID equals cover; local authority-cache invalidation rejects verify_current; FD alone makes normal detach EBUSY with unchanged visible identity; readonly MAP_SHARED mapping survives FD close and still makes detach EBUSY; unmap permits normal detach, preserves the pre-cover FUSE kernel inode/mount reference, removes the exact cover from mountinfo and performs normal outer unmount/join with no fixture leftovers. Existing FD/map readability is explicitly a limitation, not immediate revocation. No secondary clone, transferred descriptor, real remote Meta revoke, production Node restart, full ON or performance claim.

Preflight once on existing afs-build: Linux/aarch64, locked offline dependencies, Rust1.95, cc/protoc/pkg-config/libibverbs, root/FUSE/mount tools, idle Cargo, ext4 target/TMPDIR, >=3GiB build free and >=1GiB fixture free (64MiB budget). Freeze all 157 compiler input hashes and identify the new test ELF from Cargo JSON. Run affected compilation/format/lint and the exact test under private mount namespace with 60s timeout. Capture command/exit/output/source and mount/process inventory. Stop on actual environment blocker; preserve failure, no environment repair. Do not rerun unchanged standard/performance cases.

Publish compact result/hash/index only; raw Cargo output outside source. Lore commit on main; normal push waits for GitHub recovery under the user decision. This guard does not close G2.12 or change G2 counts.

Test-observer correction: simulated Meta intentionally rejects grant reacquisition after cache invalidation; both fresh target lookup and retained-FD fstat invoke FUSE authorization and return ENOSYS. Preserve both failed attempts; use the prevalidated directory FD kernel fdinfo and cover/outer mountinfo to observe detach without granting fresh authority. This is a test-observer correction, not a product repair.
