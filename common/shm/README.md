# dms-shm

`dms-shm` 集中封装 Linux 的 `memfd`、`mmap`、`munmap`、`dup`、
`sendmsg` 和 `recvmsg`。这里承担原生资源与系统调用的 unsafe 实现；
SDK TransferEngine 和 Node Arena 仍有少量显式允许的 unsafe 调用边界，
负责证明访问范围、生命周期和读写排他等前置条件，不是“调用方完全没有 unsafe”。
其余代码遵守 workspace 默认的 `unsafe_code = deny`，不能为方便扩大例外范围。

Trust boundary:

- The fd broker is for same-host, same-UID or otherwise same-trust-domain
  processes.
- Passing a file descriptor is not sandbox isolation. The receiver can access
  the mapped bytes for as long as it keeps the fd or mapping alive.
- Broker tokens are one-time capabilities. They protect against accidental
  misuse and stale descriptors, not against an untrusted local attacker with
  OS-level access to the broker socket.

Non-Linux platforms return `Unsupported`. Authoritative build, test and runtime
validation for this crate must run inside the `dms-dev` Linux VM.
