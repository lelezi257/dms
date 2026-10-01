# Dedicated VM network preparation

## Result and scope

**Local probe regression PASS; bounded live network preparation PASS.**
The [Linux semantic audit](semantic-final/semantic-result.json) records **45 PASS,
0 FAIL**. This is not full ENV qualification, a product RPC authorization test,
RoCE/verbs proof, a performance result or formal REL-10 acceptance. The
environment evaluator still blocks its deferred network predicate; the lock
remains **PREPARING**, all **69 formal cases remain NOT_RUN**, and delivery stays
active. Acceptance thresholds are unchanged.

Four independent ARM64 Linux guests use fixed addresses 192.168.109.11–14.
Twelve ordered non-self pairs each complete TCP, UDP and mutually authenticated
TLS with explicit source binding and exact fresh 32-byte data: **36 positive
protocol checks**. Every pair also rejects an untrusted server CA, wrong server
hostname and missing client certificate: **36 TLS negative checks**. A→B rejects
an untrusted client certificate separately. These counts overlap the 45 audit
predicates and are not additive test-method totals.

## Fault and cleanup

An A→B DROP affects only probe ports 19566/19567, TCP/UDP and the exact source
and destination addresses. An independent 25-second cleanup watchdog starts
before injection. All three intended exchanges fail within bounded timeouts;
actual TCP and UDP rule hit counters are nonzero. Fresh exchanges pass after
rule removal using the same running probe identities. Existing AFS process
PID/start time/executable/configuration, mounts and firewall rules match their
before snapshots. All temporary probe listeners are subsequently stopped.

The [initial semantic audit](semantic-original/semantic-result.json) has
**43 PASS, 2 FAIL**: iptables-nft materialized an empty filter table which rule
removal left behind. [Guarded cleanup](afs-v67-clean-filter.py) verified that the
table was newly created, contained only default accept chains and had no rules
or sets before deleting it. The first cleanup assertion expected three chains
but only INPUT existed; that failure is retained. Final snapshots and the
45-check audit prove restoration after corrected cleanup. The watchdog alone
did not restore the complete firewall representation. This fault prototype is
not a reusable production driver; prefer a run-scoped nftables table for the
next driver rather than weakening the exact-restoration check.

## Regression and frozen identities

- [First local run](local/original/local.log): 8 methods, one failed fixture;
  the reset exception fixture omitted its errno. Corrected fixture and startup
  cleanup regressions yield [10 PASS](local/local-r2.log).
- [First real attempt](orchestration-failure.json) retains the missing-client
  TLS negative failure and per-guest raw observations under `attempt-1/`.
  TLS 1.3 can send a certificate alert on the first application read, after
  handshake completion. The [original regression](local/tls13-original/original.log)
  fails because the alert was flattened into a generic socket failure. Preserve
  typed SSL errors and the [affected module passes 11 methods](local/local-r3.log).
- [Bind cleanup regression](local/bind-original/original.log) retains one method
  with two failed TCP/UDP subcases. Setup failures now close the newly created
  socket. The [targeted replay](local/targeted-r4-corrected.log) and
  [12-method module](local/local-r4.log) pass; Linux py_compile also passes.
  The first targeted command used the wrong class name and failed before
  execution; [its log](local/targeted-r4.log) remains, not counted as PASS.
- [Final local inputs](local/final-inputs-r4.sha256) bind the last setup-error
  fix. The wire matrix used the preceding probe frozen under each guest's
  `logs/probe-input.sha256`, matching `semantic-final/env_network.py`.
  Those live results remain under their original identity. They are reused
  for unchanged exchange paths, not relabeled as a full wire rerun of the last
  failure-only cleanup edit. No repeated full Rust or network matrix was run.
- [Static review](static-review.md) is read-only analysis, not runtime evidence.

All execution, fault injection and Python tests occur in Linux. Host JavaScript
only orchestrates Lima and collects evidence. Tested state/certificates are in
guest ext4; isolated probe credentials do not replace product credentials.
Evidence includes public certificates only, never private keys. Original
failures are retained under their own source and input hashes.

The [Rust reuse check](rust-reuse.json) binds 143 unchanged compile inputs to the
original v64 full gate. Product Rust, RPC, formats and dependencies are unchanged.
This batch adds only a standalone acceptance probe and its tests; it does not
modify `runner.py`, `environment.py`, the case manifest or environment lock.
AGENTS and the handoff remain unchanged. Full POSIX, durable backend, RDMA,
performance, 8 GiB and long stability matrices retain their scheduled stages.

The [artifact manifest](artifacts-manifest.json) binds all captured files. Raw
commands, ready identities, public certificates, exchanges, firewall hits and
stop snapshots live under `attempt-2/`, `linux/` and `semantic-final/`. The final
audit script is [retained separately](afs-v67-validate-final.py); its absolute
scratch path is the original Linux execution path, not a portable CLI contract.
