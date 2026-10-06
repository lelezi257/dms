# Mixed append diagnostic follow-up

The original r6 FAIL remains immutable: third FUSE append returns SEEK_CUR24,
expectedEOF38; it aborts before final content or concurrent checks. Same
Rust6d51aeb/map66dbbe3e, package and Linux guest kernel; no Rust/vendor change.

1. Keep all four sequential offset predicates, fsync/full dual-reopen content,
   and 128 complete unique concurrent records. Save each independent sub-result.
2. Defer only the offset failure until the bounded content/concurrent checks
   finish. Do not add fstat/getattr/SEEK_END before the third write or alter
   paths/calls to make the predicate pass. Any offset/data error keeps FAIL.
3. Run only `--semantics-groups append` with native/native positive control,
   fresh clean trusted rootfs, exact identity and normal cleanup. Preserve
   tool SHA/reversible delta and originals, not a duplicate source snapshot.

Do not switch to writeback cache, disable O_APPEND, invent a successful lseek
callback, or patch fuser/kernel to make a small test green. This diagnoses a
necessary ON gap; it does not qualify ON, performance or reopen G1. The next
independent acceptance item proceeds when this bounded distinction is resolved.
