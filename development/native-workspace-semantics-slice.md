# Short mixed-path workspace semantics

2026-10-07, after the [bounded actual lifecycle](evidence/20261007-managed-workspace/README.md). Use unchanged Linux source6d51aeb/map66dbbe3e/release package and official runc1.5.2 on afs-g2-micro; only maintained Python tools change. Fresh trusted rootfs and fixture; no Rust rebuild or unchanged standard/ordinary performance repeat.

Within one real managed container, run native/native positive control first, then FUSE/native paths exposing the same workspace. Record source/mount/process identities, tool SHA, commands, stdout/stderr and exit. Each group has its own PASS/FAIL; no failure is excluded after measurement. A failed positive control prevents the mixed-path claim. Native/native PASS cannot prove mixed coordination.

| Independent group | Fixed small pass line |
| --- | --- |
| Classic locks | Existing locks_smoke.py on both paths, child bound5seconds: conflicting byte-range lock, GETLK, unblock and close/exit release; BSD checks retained separately in the raw suite |
| Append/SEEK_CUR | Two independent descriptors/processes append fixed complete unique small records via both paths; exact count/content without overwrite, correct current offsets, fsync and reopen |
| mmap/watch | One4KiB MAP_SHARED file, native map→FUSE read and FUSE write→native map exact bytes; each path's inotify view observes the selected create/write/rename with fixed2second bound |
| Permissions/errors | Searchable probe directory; root0600 sentinel rejects container501 read/write for permission, hash remains exact; missing ENOENT and exclusive creation EEXIST; controller still permits normal stop |

The driver invokes the optional Linux semantics probe before stop. On failure it retains all group results and still normally stops/deletes/unmounts the selected fixture. No force/lazy cleanup. The declared result covers only these short boundaries, not production READY, graceful drain/restart/revocation, full POSIX, remote watch, long concurrency or performance. Real environment blockers stop only this item for help; product failures are preserved as FAIL.

Next performance output remains64MiB/C1 OFF/ON/ext4 read, sync-write and small metadata paired data with content/cleanup proof. A failed necessary semantic gate does not qualify usable ON or G2.13; it is an explicit remaining item, not a reason to reopen G1 or tune ordinary performance.

**Measured result:** [mixed-path evidence](evidence/20261007-managed-semantics/README.md) preserves the failures and positive controls. Explicit `--groups` / driver `--semantics-groups` selects an affected-only rerun; every unselected group retains its original version and result, and selected PASS never qualifies the whole four-group suite. Lock waiting requires no early ACQUIRED event under an actual bounded read; a live child alone is insufficient.
