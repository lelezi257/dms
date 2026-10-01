# Independent cross-VM verbs preparation

- Scope: RDMA-01 / ENV-01 prerequisites. The real lock remains PREPARING and all formal cases retain NOT_RUN.
- Product Rust, RPCs, public interfaces, data formats and dependencies are unchanged. Reuse the frozen v64 143-input source gate; this batch needs Linux probe/parser regression, compilation and actual cross-VM operation evidence.
- Use installed upstream rdma-core `rping`, explicitly source-bound, with debug/verbose/validation enabled. Pin its binary, guest, kernel, provider, route, GID, MTU, process and command identities.
- Start with A→B 256-byte, three-iteration stock observation. Preserve original raw logs. Then run the collector on all twelve non-self ctl/A/B/C pairs at 256 bytes and three iterations, plus A→B at 65535 bytes. The larger payload is a boundary diagnostic, not a performance run.
- Verify complete READ and WRITE payloads, including the trailing NUL, using SHA256 and expected bytes. SEND carries MR descriptors/go-ahead control in this tool; pair the client descriptor fields with server receive fields and successful completions. Do not describe SEND as a bulk-file payload test.
- Each temporary process has a bounded deadline and owned cleanup. Preserve existing AFS processes, mounts and QPs; no global RXE/firewall changes. Capture before/after resources and prove no probe-owned residual process/QP.
- Root alone runs VM commands, Python and probes. An executor owns the two new collector/test files; independent read-only review follows before publication. No automatic handoff refresh or full Rust/POSIX/performance/8 GiB/soak rerun.
- This batch prepares independent verbs observations. Connecting these observations to the conservative ENV evaluator is a separate affected-tool integration. Generic exit zero or a summary PASS cannot promote ENV.
