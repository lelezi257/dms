# AFS acceptance runner contract

`runner.py` is a conservative dispatcher. It can report a selected smoke or
full run as `PASS`, but `summary.full_release_gate_pass` is only true when the
full profile covers every active manifest case, every required matrix value,
and the frozen lock matches observed identities.

## CLI

```bash
python3 runner.py \
  [--case ID ...] [--category CATEGORY] \
  [--backend OwnerFs|DFS] [--meta etcd|Redis] [--transport MODE] \
  [--profile smoke|full] [--timeout SECONDS] \
  [--cases cases.json] [--lock acceptance.lock.json] \
  [--identity-attestation observed-identity.json] \
  [--results-dir DIR]
```

Driver commands in `cases.json` must be argv arrays. Shell strings are rejected.
The runner executes drivers with `shell=False`, streams stdout/stderr directly
to the immutable run directory, and terminates the whole process group on
timeout.

## Frozen lock identity

For release/full gates, the lock must be `FROZEN`, have
`verification.status=PASS`, and contain comparable identities for:

- `runner.sha256`
- `manifest.sha256`
- `source.sha256` or `source.git_commit`
- `binary.sha256`

`runner` and `manifest` are always hashed by the runner. `source` and `binary`
must be observed from `--identity-attestation` paths. Attested digest or git
fields are ignored; the input is only a locator.

- `source.path` must be either a readable reproducible source archive/file,
  hashed by the runner, or a readable git directory. Git source identity requires
  `git rev-parse HEAD` and `git status --porcelain`; dirty source trees block
  full release.
- `binary.path` must be a readable package or binary file hashed by the runner.
  Future drivers still need to prove they executed the intended binary; a
  declared digest alone is not observed identity.

Missing paths, unreadable paths, unsupported directories, empty identity
dictionaries or mismatched observed digests block full runs.

## Driver proof

The final non-empty JSON object on driver stdout is the proof. Minimum shape:

```json
{
  "case_id": "FUN-01",
  "profile": "smoke",
  "matrix": {"backend": "DFS", "meta": "etcd", "transport": "TCP"},
  "status": "PASS",
  "checks": [
    {"name": "digest", "status": "PASS", "evidence": "sha256 matched"}
  ]
}
```

Each `PASS` check needs a non-empty `name` and either a non-blank evidence string,
a non-empty structured object/list, or a non-empty run-local artifact file.
Boolean/numeric evidence, empty files and directories do not prove a check.
Artifact paths must stay inside the run directory.

`EXCLUDED` at proof or check level must include an `exclusion_id` that matches a
manifest `approved_exclusions` entry or an `exclusions` record with
`approved: true` or `pre_reviewed: true`. Excluded checks are reported in
accounting and do not silently disappear.

## Driver-owned matrix coverage

The runner expands only `backend`, `meta` and `transport`. Other manifest axes
are driver-owned. A PASS proof must cover each expected value explicitly:

```json
{
  "coverage": {
    "profile": "full",
    "axes": {
      "sizes_full": {
        "values": ["0", "1", "4KiB"],
        "checks": {
          "0": "size-0",
          "1": "size-1",
          "4KiB": "size-4KiB"
        }
      }
    }
  }
}
```

Every expected value must be present, every value must reference an existing
`PASS` or approved `EXCLUDED` check, and unexpected values are rejected. Profile
specific axes are scoped by name: `*_full` applies only to `--profile full` and
`*_smoke` applies only to `--profile smoke`.
