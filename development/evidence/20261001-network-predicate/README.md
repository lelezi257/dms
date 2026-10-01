# Hash-bound network preparation predicate

## Result

**Local regression PASS; affected acceptance-tool batch PASS.** Linux ARM64
validates 34 network/metadata test methods and 26 runner methods, **60 distinct
methods**. Necessary Python compilation passes. This batch changes acceptance
tools only. The unchanged 143 Rust inputs retain the original v64 source gate;
no new full Rust, POSIX, performance, 8 GiB or stability run is claimed.

The [actual preparation report](linux/preparation.json) records **36 PASS,
10 BLOCKED, 0 FAIL**, overall **BLOCKED**. Only the network preparation predicate
replaces its former deferred check. Missing initial host reserve and nine
other semantic prerequisites remain blocked. The real lock remains PREPARING,
all 69 formal cases remain NOT_RUN, and the overall delivery goal is active.

## Evidence and contract

The [bundle](bundle/bundle.json) binds 124 artifact references, reusing v66
metadata and v67 wire observations under their original identities. The
predicate checks the supported frozen probe, four guest/server identities,
twelve directed TCP/UDP/mTLS exchanges, typed TLS rejection, the actual
untrusted-client invocation, exact DROP scope/hit counters, bounded fault
failure, restored exchanges and preserved process/mount/firewall state.
Command transcripts require literal complete invocations and ordered fault
steps; summary PASS records are insufficient. Missing evidence is BLOCKED,
tampering or semantic contradiction is FAIL.

The [consumer integration](linux/consumer.json) proves that this network PASS
still produces ten qualification errors. An explicitly synthetic READY/FROZEN
dispatcher fixture uses the real evaluator and runner without mocking the
environment consumer: full dispatch stays BLOCKED, its driver does not execute,
and `full_release_gate_pass` remains false. Its synthetic source/binary
identities do not qualify a product build or environment.

The supported wire collector has SHA256
`3db932a4c1a72d450edbcc222ae8ae4010061fac84b5c012f061c91186e79612`.
The declared fault recipe has SHA256
`a485c56bf184087f4cbdc2b3dc63b3b3085a537f43f743a2b992ba4c78813353`.
Its historical executed-file digest was not captured and is not retroactively
attested. Actual counters, failed exchanges, command sequence and restoration
provide bounded fault preparation evidence. Main-flow restoration occurred
before the watchdog; its completion marker does not prove autonomous recovery.
Old observations are not current live qualification. Product RPC trust,
RoCE/verbs, formal REL-10 and the full ENV matrix remain separate.

## Regression history

- [Original evaluator](original/original.log): nine methods retain 21 failing
  assertions before network predicate implementation.
- The initial implementation's [30-method local run](review-original/local.log)
  and [26-method runner run](review-original/runner.log) passed. Static review
  found additional schema/scope gaps; those results do not qualify the fixes.
- [Review regressions](review-original/regression.log): five methods reproduce
  **13 failing assertions and three errors**. They expose malformed wrong-client
  nesting, missing preserved fields, substring address/port/scope matching,
  shell suffixes, missing negative invocation and fault nonce/timeout gaps.
- Complete token matching, typed observations, anchored rule parsing and
  explicit negative schema close those gaps. [Final 34-method run](linux/final-local.log),
  [affected 26-method runner run](linux/hardening-runner.log) and
  [compilation](linux/final-compile.exit) pass. The only subsequent test edit
  supports the research mirror's fixture path; its 34-method rerun passes.
- [First consumer check](consumer-original/consumer.log) fails because the
  harness expected a literal word in the runner reason. The reason already
  contains all ten actual blockers. The corrected check asserts those exact
  blockers; [its run](linux/consumer.log) passes. The runner was not changed.
- [Independent static review](static-review.md) has no final blockers; it is
  read-only review, not runtime evidence.

[Final inputs](linux/final-inputs.sha256), [selection](selection.json),
[Rust reuse](rust-reuse.json), [protected files](protected-files.json) and
[artifact hashes](artifacts-manifest.json) preserve the validation scope.
All test/probe execution is Linux-only; host hashing and copying are metadata
operations. Handoff and AGENTS remain unchanged. No private keys are published.
