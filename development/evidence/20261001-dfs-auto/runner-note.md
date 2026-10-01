# Runner observations

- The first archive selected the working directory recursively and included
  ignored historical files. A deleted open archive and its own transfer briefly
  consumed build-VM space. Only those exact transfer processes were terminated;
  closing their deleted-file handles restored free space. No historical
  experiment data or live service was removed.
- Subsequent candidates use `git ls-files -z` and tracked-file tar input;
  archives are about57MiB. Linux extraction accepts macOS PAX creation-time
  metadata warnings without treating them as content evidence.
- Lima tries host cwd/home paths before the explicit guest working directory;
  its `cd` warnings are retained. Commands then select the frozen Linux path and
  return the recorded test exits. A warning is not a successful test receipt.
- r1/r2 selected module runs are separate source identities. r3 adds the
  required-mode read rejection assertion and runs only its selected integrations
  before the single final full source gate. The initial required-negative
  fixture rejected only writes; no earlier read-negative PASS is claimed.
