# DMS fuser patch

This directory vendors `fuser 0.16.0` under its original MIT license.

- Upstream repository: <https://github.com/cberner/fuser>
- Upstream crate revision recorded by Cargo: `d39b15200d2509db6bf712346d2cceade3a3f2fd`
- Original license text: `LICENSE.md`
- Replacement declaration: root `Cargo.toml` `[patch.crates-io]`

DMS changes exactly one provider boundary:

- expose the already-decoded `FUSE_INTERRUPT` request through `Filesystem::interrupt`;
- dispatch that callback instead of returning `ENOSYS` unconditionally;
- allow this kernel-generated request through the owner ACL even when its header does not
  carry the mount owner's uid.

DMS uses the callback to cancel a distributed blocking `F_SETLKW` at Meta. Without it,
an interrupted process can leave a waiter that later acquires a ghost lock. Remove this
patch when the selected upstream `fuser` release provides an equivalent callback.
