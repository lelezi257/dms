# DFS cohort failure evidence and drain repair

2026-10-07 independent tooling item; base main27b0fa9c. [Original failed
product attempt](../20261007-dfs-r3-multinode-runtime/README.md) remains immutable.
No product service, performance run, Rust build or vendor edit in this item.

The old worker lost its complete failed C sample before writing its summary.
A new Linux regression reproduced that as `KeyError: sample`. The worker now
writes `probe-sample.json` and retains sample/identity/cohort in the error summary
before C_DONE and failure checks. rc/stdout/stderr/error survive failed probes
and later postchecks. No success, barrier or content check is weakened.

[Maintained acceptance relay](../../acceptance/dfs_cohort_relay.py) replaces the
unversioned transport logic for the next fresh attempt. The frozen old transport
stays in its original raw archive. The relay accepts exact ctl/A/B/C command
arrays, routes JSONL controls, retains all raw stdout/stderr/events and each exit
receipt. Coordinator rejection is preserved as the primary error; BrokenPipe
on stdin-close is secondary. Slow workers can finish and emit their tail output
while product mounts stay available. Timeout termination and partial launch are
bounded and always produce failed closure receipts. A transport-only PASS is
not a data, concurrency, performance or remote-PID acceptance result.

**Linux PASS:**10 worker guards +6 real-process relay guards: successful routing,
coordinator rejection with slow-worker tail, injected stdin-close BrokenPipe,
invalid stdout, hung cohort timeout, partial launch failure. All actually started
local test processes reaped and stdout pumps closed. CLI help also passes.
[Commands/results](summary.json), [exact input SHA](source-inputs.json),
[raw archive/member SHA](raw-archive.json). Old missing C stderr/final relay
receipts cannot be recreated by this fix and remain explicit in the original
packet. No unchanged product suite is rerun.

For the next new candidate/fixture, its host controller writes commands.json
with `ctl`, `A`, `B`, `C` mapped to exact preflighted argv (including Linux timeout,
worker/coordinator source identity, tuple/session and fresh result directories),
then uses:

```sh
python3 -B development/acceptance/dfs_cohort_relay.py \
  --commands commands.json --output fresh-cohort-receipts \
  --timeout 95 --drain-timeout 65
```

The CLI can orchestrate Lima on the host; all product timing and file operations
remain Linux. `output` must be fresh. Runtime owner verifies protocol/data with
the qualified worker/coordinator, and separately verifies exact remote worker
PID/argv completion before stopping product services. Reaped Lima/SSH proxies
alone do not prove remote worker closure. A timeout/unknown remote state stops
that item with evidence; it cannot authorize data PASS or blind fixture reuse.

G1 historical8/8 stays closed; G2.24 runtime remains failed/unaccepted. Next is
the separate targeted DFS same-parent create conflict recovery. Its Rust work
is not included in this tooling result and must qualify independently before
building a new trial candidate. No full POSIX,3FS or G2.27 completion claim.
