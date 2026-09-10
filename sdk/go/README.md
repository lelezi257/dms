# Go SDK

Module path: `github.com/lelezi257/dms/sdk/go`.

This package exposes the native Go DMS client API. It keeps protobuf, gRPC
payload descriptors, Unix FD transfer, and mmap bookkeeping inside `internal/`
packages; applications import only package `dms`.

Implemented first-batch API:

- `Connect(ctx, endpoint, ClientOptions)` and `ConnectWithOptions(ctx, ClientOptions)`
- `Set`, `SetWithOptions`
- `Get`, `GetWithOptions`
- `Del`
- `Stat` and `Scan`
- `Close`

`Get` returns caller-owned bytes and a `found` boolean. Missing keys are
`found=false, err=nil`; transport, timeout, validation, and server failures are
native `DmsError` values. The SDK does not keep a cross-request value cache.
The public inline threshold defaults to 64 KiB; gRPC send/receive message limits
match the runtime control-plane budget of 16 MiB.
