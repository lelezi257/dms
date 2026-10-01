# Independent read-only review

Initial review of 76ce65a requested six fixes: malformed untrusted-client
nesting, absent preserved keys comparing equal, substring DROP scope, trailing
shell operations, missing untrusted-client transcript, and undervalidated
negative wrapper. It also requested fault nonce/timeout schema coverage.

Root retained five-method Linux reproduction: 13 failures and three errors.
The final worktree uses guarded schema parsing, typed preservation records,
anchored complete rule matching, complete collector command tokens, the exact
untrusted-client command and wrapper, and nonce/timeout checks. The independent
reviewer found no remaining blockers in this bounded slice.

Review was static only: no edits, Python execution, VM actions or publication
by the reviewer. Fresh Linux execution is recorded separately. This review
does not assess or qualify complete ENV, product authorization or verbs.
