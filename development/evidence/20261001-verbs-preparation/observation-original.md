# Initial observation failure

The first protected-process observation was invoked without sudo on all four
guests. It failed before writing an observation with:

```
PermissionError: [Errno 13] Permission denied: '/proc/1/exe'
```

No process, mount or RDMA device was modified. The helper was not changed to
ignore inaccessible processes. Running the same read-only helper as root
provides the required complete process visibility. These initial failures do
not represent a probe or product PASS.
