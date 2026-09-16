//! Linux POSIX ACL xattr 的有界解析与 mode 同步。
//!
//! ACL 仍然是 Meta catalog 中的普通 xattr；本模块只负责校验其稳定二进制格式，
//! 以及 POSIX 要求的 ACL/mode 联动，不拥有缓存或持久化状态。

use std::collections::HashSet;

use super::{InodeId, InodeKind, XattrUpdate};

pub(crate) const ACL_ACCESS_NAME: &[u8] = b"system.posix_acl_access";
pub(crate) const ACL_DEFAULT_NAME: &[u8] = b"system.posix_acl_default";

const ACL_VERSION: u32 = 0x0002;
const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_GROUP: u16 = 0x08;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
const ACL_UNDEFINED_ID: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AclError {
    InvalidEncoding,
    DefaultAclRequiresDirectory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AclEntry {
    tag: u16,
    perm: u16,
    id: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PosixAcl {
    entries: Vec<AclEntry>,
}

impl PosixAcl {
    fn decode(value: &[u8]) -> Result<Self, AclError> {
        if value.len() < 4 || !(value.len() - 4).is_multiple_of(8) {
            return Err(AclError::InvalidEncoding);
        }
        let version = u32::from_le_bytes(value[0..4].try_into().expect("four ACL bytes"));
        if version != ACL_VERSION {
            return Err(AclError::InvalidEncoding);
        }
        let mut entries = Vec::with_capacity((value.len() - 4) / 8);
        for raw in value[4..].chunks_exact(8) {
            let entry = AclEntry {
                tag: u16::from_le_bytes(raw[0..2].try_into().expect("ACL tag")),
                perm: u16::from_le_bytes(raw[2..4].try_into().expect("ACL perm")),
                id: u32::from_le_bytes(raw[4..8].try_into().expect("ACL id")),
            };
            if entry.perm > 0o7
                || !matches!(
                    entry.tag,
                    ACL_USER_OBJ | ACL_USER | ACL_GROUP_OBJ | ACL_GROUP | ACL_MASK | ACL_OTHER
                )
            {
                return Err(AclError::InvalidEncoding);
            }
            entries.push(entry);
        }
        let acl = Self { entries };
        acl.validate()?;
        Ok(acl)
    }

    fn validate(&self) -> Result<(), AclError> {
        let count = |tag| self.entries.iter().filter(|entry| entry.tag == tag).count();
        if count(ACL_USER_OBJ) != 1 || count(ACL_GROUP_OBJ) != 1 || count(ACL_OTHER) != 1 {
            return Err(AclError::InvalidEncoding);
        }
        let has_named = self
            .entries
            .iter()
            .any(|entry| matches!(entry.tag, ACL_USER | ACL_GROUP));
        if count(ACL_MASK) > 1 || (has_named && count(ACL_MASK) != 1) {
            return Err(AclError::InvalidEncoding);
        }

        // 非命名条目必须使用 Linux 约定的 undefined id；命名条目必须携带真实
        // uid/gid，且同一 ACL 中不得重复。否则 Native API 可以绕过内核校验，
        // 在 Meta 中留下不同客户端解释不一致的 ACL。
        let mut named_users = HashSet::new();
        let mut named_groups = HashSet::new();
        for entry in &self.entries {
            match entry.tag {
                ACL_USER | ACL_GROUP if entry.id == ACL_UNDEFINED_ID => {
                    return Err(AclError::InvalidEncoding);
                }
                ACL_USER if !named_users.insert(entry.id) => {
                    return Err(AclError::InvalidEncoding);
                }
                ACL_GROUP if !named_groups.insert(entry.id) => {
                    return Err(AclError::InvalidEncoding);
                }
                ACL_USER_OBJ | ACL_GROUP_OBJ | ACL_MASK | ACL_OTHER
                    if entry.id != ACL_UNDEFINED_ID =>
                {
                    return Err(AclError::InvalidEncoding);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.entries.len() * 8);
        out.extend_from_slice(&ACL_VERSION.to_le_bytes());
        for entry in &self.entries {
            out.extend_from_slice(&entry.tag.to_le_bytes());
            out.extend_from_slice(&entry.perm.to_le_bytes());
            out.extend_from_slice(&entry.id.to_le_bytes());
        }
        out
    }

    fn effective_mode(&self, preserved: u32) -> u32 {
        let permission = |tag| {
            self.entries
                .iter()
                .find(|entry| entry.tag == tag)
                .map_or(0, |entry| u32::from(entry.perm))
        };
        let group = self
            .entries
            .iter()
            .find(|entry| entry.tag == ACL_MASK)
            .map_or_else(|| permission(ACL_GROUP_OBJ), |entry| u32::from(entry.perm));
        (preserved & !0o777)
            | (permission(ACL_USER_OBJ) << 6)
            | (group << 3)
            | permission(ACL_OTHER)
    }

    fn apply_mode(&mut self, mode: u32) {
        let has_mask = self.entries.iter().any(|item| item.tag == ACL_MASK);
        for entry in &mut self.entries {
            entry.perm = match entry.tag {
                ACL_USER_OBJ => ((mode >> 6) & 0o7) as u16,
                ACL_MASK => ((mode >> 3) & 0o7) as u16,
                ACL_GROUP_OBJ if !has_mask => ((mode >> 3) & 0o7) as u16,
                ACL_OTHER => (mode & 0o7) as u16,
                _ => entry.perm,
            };
        }
    }

    fn inherit(&mut self, requested_mode: u32) {
        let has_mask = self.entries.iter().any(|entry| entry.tag == ACL_MASK);
        for entry in &mut self.entries {
            let requested = match entry.tag {
                ACL_USER_OBJ => (requested_mode >> 6) & 0o7,
                ACL_MASK => (requested_mode >> 3) & 0o7,
                ACL_GROUP_OBJ if !has_mask => (requested_mode >> 3) & 0o7,
                ACL_OTHER => requested_mode & 0o7,
                _ => 0o7,
            };
            entry.perm &= requested as u16;
        }
    }
}

pub(crate) fn validate_acl_xattr(
    name: &[u8],
    value: &[u8],
    kind: InodeKind,
    current_mode: u32,
) -> Result<Option<u32>, AclError> {
    if name == ACL_DEFAULT_NAME && kind != InodeKind::Directory {
        return Err(AclError::DefaultAclRequiresDirectory);
    }
    if name != ACL_ACCESS_NAME && name != ACL_DEFAULT_NAME {
        return Ok(None);
    }
    let acl = PosixAcl::decode(value)?;
    Ok((name == ACL_ACCESS_NAME).then(|| acl.effective_mode(current_mode)))
}

pub(crate) fn access_acl_after_chmod(value: &[u8], mode: u32) -> Result<Vec<u8>, AclError> {
    let mut acl = PosixAcl::decode(value)?;
    acl.apply_mode(mode);
    Ok(acl.encode())
}

pub(crate) fn inherit_default_acl(
    inode: InodeId,
    kind: InodeKind,
    requested_mode: u32,
    parent_default: Option<&[u8]>,
) -> Result<(u32, Vec<XattrUpdate>), AclError> {
    let Some(default_value) = parent_default else {
        return Ok((requested_mode, Vec::new()));
    };
    let default_acl = PosixAcl::decode(default_value)?;
    let mut access_acl = default_acl.clone();
    access_acl.inherit(requested_mode);
    let mode = access_acl.effective_mode(requested_mode);
    let mut updates = vec![XattrUpdate {
        inode,
        name: ACL_ACCESS_NAME.to_vec(),
        value: Some(access_acl.encode()),
    }];
    if kind == InodeKind::Directory {
        updates.push(XattrUpdate {
            inode,
            name: ACL_DEFAULT_NAME.to_vec(),
            value: Some(default_acl.encode()),
        });
    }
    Ok((mode, updates))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_acl(user: u16, group: u16, other: u16) -> Vec<u8> {
        PosixAcl {
            entries: vec![
                AclEntry {
                    tag: ACL_USER_OBJ,
                    perm: user,
                    id: u32::MAX,
                },
                AclEntry {
                    tag: ACL_GROUP_OBJ,
                    perm: group,
                    id: u32::MAX,
                },
                AclEntry {
                    tag: ACL_OTHER,
                    perm: other,
                    id: u32::MAX,
                },
            ],
        }
        .encode()
    }

    #[test]
    fn access_acl_updates_mode_bits() {
        let value = basic_acl(0o7, 0o5, 0o1);
        assert_eq!(
            validate_acl_xattr(ACL_ACCESS_NAME, &value, InodeKind::RegularFile, 0o1000),
            Ok(Some(0o1751))
        );
    }

    #[test]
    fn directory_default_acl_is_inherited_and_masked_by_create_mode() {
        let default = basic_acl(0o7, 0o7, 0o7);
        let (mode, updates) =
            inherit_default_acl(9, InodeKind::RegularFile, 0o640, Some(&default)).expect("inherit");
        assert_eq!(mode & 0o777, 0o640);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, ACL_ACCESS_NAME);
    }

    #[test]
    fn named_acl_requires_mask_and_unique_concrete_identity() {
        let acl = |named_id, include_mask| {
            let mut entries = vec![
                AclEntry {
                    tag: ACL_USER_OBJ,
                    perm: 0o6,
                    id: ACL_UNDEFINED_ID,
                },
                AclEntry {
                    tag: ACL_USER,
                    perm: 0o4,
                    id: named_id,
                },
                AclEntry {
                    tag: ACL_GROUP_OBJ,
                    perm: 0,
                    id: ACL_UNDEFINED_ID,
                },
            ];
            if include_mask {
                entries.push(AclEntry {
                    tag: ACL_MASK,
                    perm: 0o4,
                    id: ACL_UNDEFINED_ID,
                });
            }
            entries.push(AclEntry {
                tag: ACL_OTHER,
                perm: 0,
                id: ACL_UNDEFINED_ID,
            });
            PosixAcl { entries }.encode()
        };

        assert!(PosixAcl::decode(&acl(1234, true)).is_ok());
        assert_eq!(
            PosixAcl::decode(&acl(ACL_UNDEFINED_ID, true)),
            Err(AclError::InvalidEncoding)
        );
        assert_eq!(
            PosixAcl::decode(&acl(1234, false)),
            Err(AclError::InvalidEncoding)
        );

        let mut duplicate = PosixAcl::decode(&acl(1234, true)).expect("valid named ACL");
        duplicate.entries.insert(
            2,
            AclEntry {
                tag: ACL_USER,
                perm: 0o2,
                id: 1234,
            },
        );
        assert_eq!(
            PosixAcl::decode(&duplicate.encode()),
            Err(AclError::InvalidEncoding)
        );
    }
}
