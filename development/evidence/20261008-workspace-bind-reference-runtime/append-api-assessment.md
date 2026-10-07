# Mixed append: no supported minimal position repair found

2026-10-08, current source5f433c3a, read-only analysis. Historical6d/r8 mixed append FAIL remains unchanged. No product rerun or Rust/vendor/kernel/config-gate edit was made for this analysis.

Facts:
- OwnerFs preserves physical O_APPEND (src/node/vfs/ownerfs.rs:4019, storage/localfs.rs:725), writes the physical fd, and FUSE replies only with actual byte count (src/node/fuse.rs:1224, vfs.rs:221). Native-eligible replies already have TTL0/DIRECT_IO; handle getattr reads physical metadata. No public notification sets an open file's current position.
- Linux stable6.8.12 fuse_direct_write_iter calls generic_write_checks before daemon WRITE, with cached FUSE inode i_size for append; WRITE advances request position by returned bytes. SEEK_CUR goes through generic_file_llseek, not daemon FUSE_LSEEK. WRITE ABI has size/padding only.
- inval_inode expires attributes, not f_pos. notify_store may extend i_size but cannot atomically coordinate native writers before that write's append position was chosen. writeback's coherent-all-writes-through-kernel assumption is incompatible with native bind writers.

Inference, not a new runtime trace: cached size12 after first FUSE append, native EOF26, then physical O_APPEND writes to38 while kernel advances12+12=24. This explains the preserved observation; exact daemon request offset was not captured in that run. Stable6.8.12 corroborates the Ubuntu lineage but does not audit its full patched build/backports.

First-party/official sources:
- https://github.com/gregkh/linux/blob/v6.8.12/fs/fuse/file.c#L1473 (direct write), #L2547 (llseek)
- https://github.com/gregkh/linux/blob/v6.8.12/fs/read_write.c#L1579 (generic append position)
- https://github.com/torvalds/linux/blob/v6.8/include/uapi/linux/fuse.h#L772 (WRITE reply)
- https://github.com/torvalds/linux/blob/v6.8/fs/fuse/inode.c#L473 (attribute expiration)
- https://github.com/torvalds/linux/blob/v6.8/fs/fuse/dev.c#L1477 (STORE)
- https://github.com/cberner/fuser/blob/v0.16.0/src/notify.rs#L64 (public notifications)
- https://github.com/libfuse/libfuse/blob/fuse-3.16.2/include/fuse_lowlevel.h#L478 (append/cache contract)
- https://docs.kernel.org/filesystems/fuse/fuse-passthrough.html and https://github.com/torvalds/linux/blob/v6.9/fs/fuse/passthrough.c#L49 (future backing-file write path; not a6.8/kernel-replacement task)

Decision: defer the actual append offset repair as a protocol capability topic; keep its FAIL/necessary gate open. Do not replace O_APPEND with len+pwrite, invent successful lseek, switch writeback, require users to fstat/SEEK_END, or label native/native as mixed-FUSE repair. This is a product/API gap, not an environment blocker.

Joint host and private runc adapter switches are deliberately rejected (src/config.rs:473; tests/config_contract.rs:326). Different mount namespaces violate the same live Home anchor requirement (ownerfs/native_home.rs:220); an already covered target also fails core unique-mount admission. Removing only that configuration gate would create an unsafe double-claim/race, not fix append. The adapter separately uses the real physical Home before container cloning; its existing module boundary remains.

The selected independent next item integrates genuine Meta epoch-error Node shutdown with held bound-file FD/mmap. Existing component drain and no-reference error closure are reused but are not proof of this previously untested runtime combination. Full bind, mixed append/locks/watch, formal performance and third-party migration remain unfinished.
