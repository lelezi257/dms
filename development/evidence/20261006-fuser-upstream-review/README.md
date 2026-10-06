# Official fuser comparison and migration boundary

2026-10-06, source e925c5b. Read-only comparison; current private vendor has not been removed. **R2 BLOCKED by missing public APIs; not complete.** No third-party source was edited in this task.

The [complete machine-readable differences](differences.json) and [full unified diff](vendored-vs-official-0.16.0.diff) compare every shipped path to the official crates.io0.16.0 archive. Packaging omissions are distinguished from changed code. This is not proof that the current vendor is original upstream.

| Path | Difference | Use/call site |
| --- | --- | --- |
| `.cargo_vcs_info.json` | omitted-official | Official crate packaging metadata omitted; provenance recorded here. |
| `.cirrus.yml` | omitted-official | Upstream CI omitted; no runtime call site. |
| `.dockerignore` | omitted-official | Upstream container metadata omitted; no runtime call site. |
| `.github/workflows/ci.yml` | omitted-official | Upstream CI omitted; no runtime call site. |
| `.gitignore` | omitted-official | Upstream repository metadata omitted. |
| `AFS-PATCH.md` | vendor-only | Private-patch description omits actual behavior changes; not an upstream file. |
| `Cargo.lock` | omitted-official | Upstream example/build lock omitted; AFS workspace lock controls dependency resolution. |
| `Cargo.toml` | modified | Path dependency and ABI7.33/7.36 feature inheritance; AFS Cargo.toml:51. |
| `deny.toml` | modified | Lint/dependency-policy difference, no product call site. |
| `examples/notify_inval_inode.rs` | modified | Upstream example adapted to private Filesystem signature; not compiled by AFS binaries. |
| `examples/passthrough.rs` | modified | Upstream example adapted to private Filesystem signature. |
| `examples/poll.rs` | modified | Upstream example adapted to private Filesystem signature. |
| `examples/simple.rs` | modified | Upstream example adapted to private Filesystem signature. |
| `src/channel.rs` | modified | Private shutdown socket/wakeup; BackgroundSession mount lifecycle in src/node/fuse.rs:60. |
| `src/lib.rs` | modified | Private Filesystem killpriv/lock/interrupt API and lint allowances; src/node/fuse.rs:721,946,1378,1416,1486. |
| `src/ll/fuse_abi.rs` | modified | Private ABI fields/constants including killpriv-v2/direct-I/O mmap/FLOCK flags; src/node/fuse.rs:564-575. |
| `src/ll/request.rs` | modified | Private request decoding of flags/killpriv/interrupt. |
| `src/mnt/fuse_pure.rs` | modified | EBUSY unmount fallback; managed unmount/join path in src/node/fuse.rs. |
| `src/reply.rs` | modified | entry_with_ttls extension; src/node/fuse.rs:614, now available upstream0.18 with changed parameter order. |
| `src/request.rs` | modified | Private dispatch to killpriv/lock-options/interrupt callbacks; src/node/fuse.rs. |
| `src/session.rs` | modified | Private receive-loop teardown/wakeup; FuseMountGuard shutdown/join. |

## Official candidate and required behavior

The preferred published candidate is exact `fuser =0.18.0`, no path dependency/private patch. It offers public `InitFlags` capability negotiation including `FUSE_DIRECT_IO_ALLOW_MMAP`, typed open flags and `entry_with_ttls` (or conservative TTL=0 fallback). It does not expose the private lock-options/interrupt or killpriv-v2 callback API AFS currently consumes. Do not advertise a capability the adapter cannot implement.

| Required public surface | Release0.18.0 | Official master c0420fc |
| --- | --- | --- |
| direct-I/O mmap InitFlags/public FopenFlags | Yes | Yes |
| independent attribute/name TTL | Yes | Yes |
| setattr/open/create kill_suid_gid cause | No | Yes |
| getlk/setlk lk_flags to distinguish flock | No | No |
| Filesystem interrupt callback/raw request public access | No | No |

Official master inspected: `c0420fc49d3f1ce09603beb127f392eb2726c2a1`. It is a fixed upstream commit, not a published release and still misses the final two required surfaces. Switching versions or deleting vendor alone cannot preserve the current lock semantics. Disabling FLOCK capability lets Linux handle locks locally and does not prove cross-node locking; ignoring interruption does not prove cancellation. The legacy kernel killpriv path must be tested against AFS authoritative metadata/permissions before claiming compatibility.

Existing regression entry points: `tests/fuse_contract.rs` killpriv fail-closed/legacy behavior, TTL0 freshness and real direct-I/O mmap; `src/node/fuse.rs` lock-kind and pending-wait tests; `src/node/vfs/locks.rs` cancellation before/after waiter registration. No tests were removed or changed to bless a dependency downgrade. No new Rust build was needed for this read-only determination.

**Next required decision:** preserve existing distributed flock/cancellation and obtain missing upstream public API, or explicitly accept a reduced lock support contract and validate that contract on an official dependency. Await user scope clarification; R1 and independent work continue. No external upstream message/issue has been sent. No private patch will be added.

Official primary sources: [0.18 release changelog](https://raw.githubusercontent.com/cberner/fuser/v0.18.0/CHANGELOG.md), [0.18 public API](https://docs.rs/fuser/0.18.0/fuser/trait.Filesystem.html), [fixed upstream lib.rs](https://raw.githubusercontent.com/cberner/fuser/c0420fc49d3f1ce09603beb127f392eb2726c2a1/src/lib.rs), [request parameter API](https://raw.githubusercontent.com/cberner/fuser/c0420fc49d3f1ce09603beb127f392eb2726c2a1/src/request_param.rs), [crates release metadata](https://crates.io/api/v1/crates/fuser).
