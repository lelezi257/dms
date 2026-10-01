# Static review and scope

An independent read-only reviewer found three improvements in the initial
archival helper: inspect all process mount namespaces, estimate reclaimable
blocks before mutation, and measure capacity on the actual destination
filesystem. These findings arrived after the actual successful migration.
The initial script and its seven Linux test results are preserved separately.

The current helper implements all three. Nine Linux regressions include foreign
mountinfo parsing, reserve rejection, active-FD rejection, copy corruption,
link-install rollback, preservation and manual restoration. Fresh verification
checks all current namespaces and archived content; protected files/processes
and mounts match their before observations. The completed migration was not
rerun under the strengthened script or relabeled as having used it.

The reviewer also attempted local Mac compile/unit checks. They were outside
the Linux validation policy, produced no valid acceptance evidence and are
excluded from all PASS counts. Only root-run Linux raw logs support the stated
regression results. Subsequent reviewers are restricted to static reads.

This is a narrow environment preparation review, not a performance, full
filesystem, formal environment or release verdict.

Follow-up static-only review of the strengthened helper, regressions and README
found no concrete blocker for this completed reserve-repair scope. It did not
run any tests/builds/VM actions. Executed Linux proof remains the separate basis
for the PASS counts.
