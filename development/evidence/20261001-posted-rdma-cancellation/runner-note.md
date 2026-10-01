# Runner notes

- r1 compile failed with E0716 in a test-only temporary path borrow. r2 binds the
  checkpoint path before select. Production inputs are unchanged.
- r2 GDB blocked its event loop while waiting inside the breakpoint handler;
  other threads could not complete their fork events. The original25-second
  timeout/unknown inferior exit and runner assertion failure remain intact.
  r3 moves the wait to the external controller and passes.
- r4 binds posted QP/TID and all phase PIDs. r5 binds the requested device. r6
  checks exact retained IDs against the complete drained inventories, preventing
  a retired-TID resource from escaping accounting. Each runner version is
  preserved. Rust tests are identical across r2..r6.
- Root-created GDB evidence directories initially rejected two user-side output
  or copy attempts. Only the identified new evidence directories were chowned;
  tests/gate were then executed. These setup errors did not require rerunning
  unchanged Rust tests. The first flattened host copy was discarded from the
  publish tree and recopied from intact Linux evidence with separate run paths.
- No original data/services were removed. C was observed idle and temporarily
  stopped before starting the separate build VM. Linux tests use guest ext4 and
  real RXE. The final VM restore check is recorded separately.
