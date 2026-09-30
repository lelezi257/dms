# host collection globals error

Status: INCONCLUSIVE / host evidence collection bug.

The first `run` invocation completed the v51 guest product probe, but the host
collector attempted to pull `probe-artifacts/20260930T231148Z-pid715695` from the
v51 runtime. That run id belonged to the prior v50 probe because the imported
helper module's `GUEST_RUN` global had not been rebound before using helper
collection functions.

The product run itself was not rerun for this note. The wrapper was fixed to
rebind imported helper globals and a `collect` action pulled the v51 raw report
from `/mnt/lima-afsadata/afs-delivery/shutdown-signal-v51`. An additional direct
copy in `guest-explicit-20260930T233254Z/` verifies the host report SHA matches
the A-side report SHA.
