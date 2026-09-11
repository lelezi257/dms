# Go SDK

Module path: `github.com/lelezi257/dms/sdk/go`.

This package exposes the native Go DMS client API. It keeps protobuf, gRPC
payload descriptors, Unix FD transfer, and mmap bookkeeping inside `internal/`
packages; applications import only package `dms`.

Implemented first-batch API:

- `Connect(ctx, endpoint, ClientOptions)` and `ConnectWithOptions(ctx, ClientOptions)`
- `Set`, `SetWithOptions`
- `Get`, `GetWithOptions`
- `GetInto`
- `GetReader`
- `SetFrom`
- `Del`
- `Stat` and `Scan`
- `Close`

`Get` returns caller-owned bytes and a `found` boolean. Missing keys are
`found=false, err=nil`; transport, timeout, validation, and server failures are
native `DmsError` values. The SDK does not keep a cross-request value cache.
The public inline threshold defaults to 64 KiB; gRPC send/receive message limits
match the runtime control-plane budget of 16 MiB.

`GetInto` copies the selected fixed-version read into a caller-provided buffer.
`GetReader` exposes the same fixed-version read as an `io.ReadCloser`; callers
must close the body when they stop early. Shared-memory reads copy directly from
the mapped Region into the caller buffer. TCP payload reads use the current
unary protobuf `Download` protocol one segment at a time.

`SetFrom` consumes exactly the caller-provided length. Small values still use one
inline RPC. Large shared-memory writes stream from the source into staging
memory. Large TCP writes must buffer the declared length for the current unary
protobuf `Upload` protocol; this is protocol-required buffering, not an SDK
value cache.
