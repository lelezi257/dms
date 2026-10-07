# Source-rejection case: precise stop scope before execution

2026-10-07. Clarification to [frozen case](native-source-rejection-slice.md), original SHA7c5d5c63e06d44499d8a40fbe834acb9d5fc46672da4d5d3a5c71e141b5077bf. No result exists yet and the original contract bytes stay unchanged.

The fixed6d controller's public stop invokes `runc kill --all <owned-container> KILL` when that managed container is running/created (`src/node/native_workspace.rs` cleanup), then verifies Stopped, exact final clone/source/namespace, normal unmount, runtime deletion and export detach. This existing behavior is not a graceful application shutdown or production revocation/drain ACK. It is not changed for this narrow request-rejection case.

“No force/lazy unmount or unrelated process cleanup” in the frozen case forbids forced/lazy filesystem detachment and touching unrelated processes; it does not impose zero SIGKILL on the controller's identity-bound own container. Save the actual runc argv and response/exit. The tiny legitimate command must finish successfully and its exact bytes must be verified before public stop. Node/Meta must still produce their actual processctl closure receipts; PID absence alone does not prove successful lifecycle cleanup.

A bounded PASS, if obtained, will establish source-field rejection, unchanged active identity, continued tiny-command usability and this exact existing cleanup sequence only. Graceful application draining, unexpected process death, restart reconciliation and full native ON remain unqualified. No published historical normal-cleanup statement is upgraded to a graceful-stop claim.
