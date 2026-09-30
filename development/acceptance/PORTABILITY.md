# Acceptance preparation inputs

This directory versions the current acceptance runner, drivers, probes, suite selectors, VM templates, comparison preparation scripts and PREPARING manifests. It contains no VM data, TLS private keys, binaries or source build caches.

Run these scripts from the research layout they expect, not directly inside this directory:

```sh
# Git checkout is <work>/source. Run from the checkout.
mkdir -p ../experiments
cp -a development/acceptance ../experiments/afs-acceptance
```

If the destination already exists, compare it before copying; do not overwrite independent work. Tests and filesystem services run in Linux. macOS may edit templates and orchestrate VMs. Read [handoff](../../docs/handoff.md) before running.

VM templates preserve the observed original host paths, guest user and local image-cache URL. Adapt those three machine-specific values on a new machine; keep image SHA, ARM64 architecture, kernel, CPU/RAM, disk and VM-role constraints. Reobserve networking and identities; the saved `acceptance.lock.json` is PREPARING and never proves the new environment. No real release driver is declared READY merely because its source exists.

The original runner and tests assume `<work>/experiments/afs-acceptance` and `<work>/source`. The default helper binary paths and guest runtime lanes also need an identified newly built candidate; never copy historical expected SHA/PID into observed results.
