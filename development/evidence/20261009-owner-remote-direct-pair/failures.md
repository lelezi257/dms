# Preserved failures and scope

- Historical 20261008 uid501-server admission and ENODEV/default-cache results retain their old version/criteria/conclusions. The artificial request for deployment permission was withdrawn: isolated fixed-SHA staging is normal authorized test preparation.
- First new preparation copied the official mfsfileinfo alias without its mfsdiagtools target. Original `owner-remote-direct-read-20261009-r1/run.stderr` preserves FileNotFoundError; the original alias is retained as `mfsfileinfo.failed-alias`. Copying the exact official target corrected preparation; zero services/formal samples before correction.
- First actual runtime: uid501 received EACCES on historical root-created Moose payload; zero warmup/formal rounds. Original failure.json/stdout/stderr, six normal exits and restoration are archived. No old file permissions were relaxed. A unique new uid501:501/0600 file inherited the existing one-copy class; original file identity/bytes retained.
- Sole formal comparison: throughput FAIL (0.914140x<1.2), independent pooled p95 PASS (0.538794x<=0.8), overall FAIL. All five samples retained, including Owner237.077MiB/s; no outlier removal or rerun. Different fresh-file ages/paths are disclosed; both physical payloads fully hot and direct FUSE, independently remote-byte-proved.
- Cumulative retained Owner logs: 32,044B,15ERRO/71WARN/zeroTRACE, including prior lifecycle history. These are cumulative counts, not claims of new-run-only errors or zero errors.
- Cleanup first encountered PermissionError reading /proc/1/exe in the unprivileged build VM process inventory; no build files were deleted before that failure. Same exact owned paths were checked and removed using existing sudo. No VM repair/reconfiguration.

Original cleanup stderr (after the raw archive was sealed):

```text
Traceback (most recent call last):
  File "<string>", line 7, in <module>
PermissionError: [Errno 13] Permission denied: '/proc/1/exe'
```

No Rust/vendor/product change, rebuild, repackaging, full POSIX run or new broad correctness claim. Owner-only performance does not qualify combined fs=all performance, writes, other sizes, cache states or concurrency. G1 historical8/8 stays closed; original G2 12/0/15 unchanged.
