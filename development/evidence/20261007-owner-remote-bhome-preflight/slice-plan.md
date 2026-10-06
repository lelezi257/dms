# Owner remote read/delete: minimal new Linux fixture

Prepared only; no deployment, workload or VM mutation. Parent controls execution. Current published OFF package source6d51aeb/map66dbbe3e/157 inputs remains fixed.

## Fixture and fresh admission

Use unique `owner-remote-6d-bhome-20261007-r1` under each dedicated ext4 data volume. ctl runs new local-file Meta and new Moose master; B runs new Owner Home and sole Moose chunkserver; A runs new Owner client, sole Moose mount and both diagnostic payloads. New Owner workspace is created on B through FUSE, then its Meta location receipt must prove B Home/session/epoch before A accesses it. Native/bind remains OFF. Do not use host backing directories as measured paths.

Proposed ports: Meta22800/22801; each-node22900/22901; Moose23040–23043. Saved `ss` inventories show them clear on all3; a fresh pre-start occupancy gate is still required. Unique root existence/exact mount, ELF/SHA/ldd, selected class tool aliases, config and TLS/TCP must be admitted once. No dependency installation, old-service cleanup or environment repair is authorized by this plan.

## Budget (case-specific, not copied whole-matrix policy)

| VM | Previous actual free | New working budget | Remaining free floor | Required available |
|---|---:|---:|---:|---:|
| ctl |4,481,642,496B|256MiB|512MiB|768MiB|
| A |4,322,222,080B|256MiB|1GiB|1.25GiB|
| B |18,827,644,928B|1GiB|4GiB|5GiB|

All pass these lower bounds in the saved inventory; fresh admission must bind actual deployment bytes. A node ELF is25,880,120B, Meta15,821,088B; payload72,400B. A does not store Home file data. B budget:256MiB deployment/state,256MiB logs/metadata,512MiB dataset/chunk/trash overhead. ctl/A budget details and byte predicates are in JSON. Two fixed64MiB files plus up to6delete cohorts100×4KiB per target retain139,132,928 logical bytes, before overhead. No simultaneous per-read copied snapshots. Record actual `du`/`df` at phase boundaries, stop new affected case if budget/floor would be exceeded; never discard raw logs to pass admission.

The historical4GiB guard was old helper RESERVE and chunkserver leave-space. Preserve4GiB on B chunk disk; do not impose it on A client/ctl master. A has4.0254GiB free, not a full disk. The unchanged A-storage helper remains stopped; this is a separately identified B fixture.

## Minimal adaptation and launch order

1. Reuse fixed published package SHA`ee25d589…`, Node`2cf1f538…`, Meta`2c7b7d08…`; full values in JSON. Copy only identified needed ELF/scripts into new roots on Linux. Reuse IO ELF`70ac97c7…`; if unavailable, compile canonical C on existing Linux build environment only and bind output SHA/provenance, without rebuilding Rust.
2. Use published `afs-trial-config cluster --backend local-file --meta-host 192.168.109.11 --node remote-a-r1=192.168.109.12 --node remote-b-r1=192.168.109.13 --meta-grpc-port 22800 --meta-rest-port 22801 --node-grpc-port 22900 --node-rest-port 22901 --output NEW_TLS_OUTPUT`. It emits fs=all/R2 and shared paths; adapt only new generated TOMLs to `fs=ownerfs`, remove `dfs_mount`, native false, guest-specific new root paths and TLS paths. Keep generated originals. Node/Meta `--print-config` exits before listener/mount creation and must prove those fields; do not launch its defaults unchanged. No new Rust/third-party change.
3. Use old helper settings as template, not driver invocation: new ctl master DATA_PATH/EXPORTS, new ports; B chunk DATA_PATH/HDD path, BIND_HOST/CSSERV_LISTEN_HOST=192.168.109.13, MASTER_HOST=192.168.109.11, `HDD_LEAVE_SPACE_DEFAULT=4GiB`, `HDD_FSYNC_BEFORE_CLOSE=1`. Immutable prefix stays `/opt/afs-moose-round3-v85`. Required precise CLI/tool SHAs derive from archived226manifest, not new website docs. Fresh master starts from pinned metadata.mfs.empty under new root; old master/state untouched.
4. Launch new Meta/master, B chunk, B+A nodes and A Moose mount. Stock argv: `mfsmaster -f -c NEW_CFG start`, `mfschunkserver -f -c NEW_CFG start`, `mfsmount -f -H 192.168.109.11 -P 23042 -o allow_other,mfsnice=0,mfscachemode=AUTO,mfstimeout=30 NEW_MOUNT`. Record exe SHA, config SHA, pid, boot/starttime, mountID/source/target, process return/logs. Moose topology must show exactly one live chunkserver atB.
5. Create unique Owner workspace on B FUSE; `GET /v1/roots/root-<hex(workspace-name)>` must prove newB id/session/epoch/endpoint serving. Create Moose class `owner-remote-1CP-20261007-r1`: public `mfscreatesclass -M MOUNT -C '*' -K '*' CLASS`; list and verify one-copy CREATE/KEEP, ARCH/TRASH keep; `mfssetsclass CLASS DIR` before file creation, then assignment/fileinfo/physical-holder proof. No setgoal/default2CP or v89 fixture mutation.

## Diagnostic case execution (Linux A only)

Canonical first-party C: `source/development/acceptance/probes/ownerfs_native_closeout_io.c`. Preparation outside performance series: `io ABS_FUSE_FILE seq-write 67108864 1048576 1 fdatasync 67108864 97 create unobserved`; check returned counts/content and parent directory sync outside timer. Timed read: `io ABS_FUSE_FILE seq-read 67108864 1048576 1 close 67108864 97 existing unobserved`. Same fixed file/pattern on both new mounts,1warmup+5alternating paired measured rounds; reuse immutable read file, no extra per-round64MiB copies. Timer includes open/thread setup/64pread/content comparison/close, excludes preliminary inspect and final stat. Cache is UNOBSERVED, no cold/hot assertion, no global drop_caches.

Reuse only `pair_delete` algorithm from `experiments/afs-acceptance/e2e-small-20261006/run-small.py:110–144`: new parameterized roots and target names,100exclusive files×4096B, fdatasync each/fsync parent and complete fresh-open content checks before timing. Time100unlink loop, then fsync parent and namespace-empty/B-Home cross-mount absence checks outside timer.1warmup+5alternating pairs; record per-sample ns/ops per second/mean unlink ns. Moose trash retention/physical reclamation is excluded, leave defaults and record policy; do not purge.

Save raw argv/stdout/stderr/rc, frozen sizes/pattern/pairs/timer boundaries, all failure/partial rows, deployment/config/location/topology/mount receipts, MiB/s and paired ratio. Correctness is independent PASS/FAIL; performance is DIAGNOSTIC_REPORTED. No wholeVM fair-isolation claim, no new formal remote-performance PASS, no reinterpretation of old32MiB range qualification or B001 strong-durable-write ACK. Normal sync-return and content predicates remain required; if Moose directory fsync/other selected predicate actually fails, retain failure and stop affected case, do not quietly remove it.

## Stop and handoff

No newly observed actual B-topology environmental blocker. Pending are actual candidate/payload/tool availability, exact generated config/TLS, fresh selected-port/collision/budget checks and authenticated connectivity. Existing A/ctl cohorts remain present; this is small diagnostic data with that limitation. At completion TERM only proven new services, verify exact mounts disappear and exitcodes, preserve all result/state evidence; no lazy unmount or old service changes. Stop only affected branch on real missing dependency/capacity/TCP/predicate failure, save reason, return to parent.

Local code anchors: `source/src/config.rs:249` supports fs=ownerfs; `source/src/bin/afs-node.rs:1` documents print-config; `source/src/node/vfs/ownerfs/root.rs:478` creates Home locally, `:1338` derives root-id; `source/src/meta/rest.rs:89` location receipt; helper configs `round3-moose-read.py:158–188`, launch`:259–267`; one-copy proof `round3-moose-policy.py:15–29,55–66`. Live [Moose chunk config docs](https://moosefs.com/manpages/mfschunkserver-cfg) describe current5.1.0; they confirm leave-space/public host settings only, not fixed4.59.2 identity. Version-specific help and archived pinned-manifest/class receipts control execution.
