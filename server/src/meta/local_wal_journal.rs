//! 本地 WAL：只负责保存 Meta 已决定的顺序，不接管 CAS、Watch 或副本放置。
//!
//! 追加成功的前提是整条 frame 已写入且 sync_data 成功。恢复时先校验完整记录，
//! 再截去崩溃留下的半条尾记录，才允许下一次追加。不能只在内存里跳过残尾。

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use crc32fast::Hasher;
use dms_protocol::v1 as pb;
use prost::Message;

use crate::filesystem::{
    XattrUpdate, dentry_from_proto, dentry_to_proto, inode_from_proto, inode_to_proto,
    namespace_result_from_proto, namespace_result_to_proto,
};

use super::metadata_journal::{
    BlockRetirementParticipant, BlockRetirementRecord, CommitSequenceRecord,
    FilesystemAttributesUpdatedRecord, FilesystemInodeCreatedRecord,
    FilesystemNamespaceMutationRecord, FilesystemOrphanReapedRecord,
    FilesystemSymlinkCreatedRecord, FilesystemVersionCommitRecord, FilesystemXattrUpdatedRecord,
    JournalEntry, JournalError, JournalRecord, MetaSnapshot, MetadataJournal,
    SnapshotBlockRetirement, SnapshotFilesystemAttributeOperation,
    SnapshotFilesystemNamespaceOperation, SnapshotFilesystemOperationVisibility,
    SnapshotFilesystemVersionOperation, SnapshotOperation, SnapshotReplica,
    SnapshotReplicaOperation, SnapshotSession, VersionCommitRecord,
};

const WAL_MAGIC: &[u8; 4] = b"DMSJ";
const SNAPSHOT_MAGIC: &[u8; 4] = b"DMSS";
const FRAME_VERSION: u8 = 1;
const HEADER_LEN: usize = 22;

const RECORD_NODE_SESSION_OPENED: u8 = 1;
const RECORD_REPLICA_ACCEPTED: u8 = 2;
const RECORD_REPLICAS_REPORTED: u8 = 3;
const RECORD_VERSION_COMMITTED: u8 = 4;
const RECORD_OPERATION_REMEMBERED: u8 = 5;
const RECORD_NODE_EVENT_ACKNOWLEDGED: u8 = 6;
const RECORD_VERSIONS_COMMITTED: u8 = 7;
const RECORD_VERSION_COMMITTED_V2: u8 = 8;
const RECORD_VERSIONS_COMMITTED_V2: u8 = 9;
const RECORD_BLOCK_RETIREMENT_PREPARED: u8 = 10;
const RECORD_BLOCK_RETIREMENT_ACKNOWLEDGED: u8 = 11;
const RECORD_BLOCK_RETIREMENT_FINALIZED: u8 = 12;
const RECORD_BLOCK_RETIREMENT_RELEASED: u8 = 13;
const RECORD_FILESYSTEM_INODE_CREATED: u8 = 14;
const RECORD_FILESYSTEM_VERSION_COMMITTED: u8 = 15;
const RECORD_FILESYSTEM_NAMESPACE_MUTATED: u8 = 16;
const RECORD_FILESYSTEM_SYMLINK_CREATED: u8 = 17;
const RECORD_FILESYSTEM_ORPHAN_REAPED: u8 = 18;
const RECORD_FILESYSTEM_ATTRIBUTES_UPDATED: u8 = 19;
const RECORD_FILESYSTEM_VERSION_COMMITTED_V2: u8 = 20;
const RECORD_FILESYSTEM_ATTRIBUTES_UPDATED_V2: u8 = 21;
const RECORD_FILESYSTEM_NAMESPACE_MUTATED_V2: u8 = 22;
const RECORD_FILESYSTEM_SYMLINK_CREATED_V2: u8 = 23;
const RECORD_FILESYSTEM_XATTR_UPDATED: u8 = 24;

/// One durable, single-process WAL directory.
pub(crate) struct LocalWalJournal {
    wal_path: PathBuf,
    snapshot_path: PathBuf,
    entries: Vec<JournalEntry>,
    snapshot: Option<MetaSnapshot>,
    next_sequence: u64,
    /// 追加/刷盘失败后，磁盘上是否留下完整记录无法仅靠返回错误判断。
    /// 因此本实例拒绝继续写；重开时由统一恢复逻辑重新确定有效尾部。
    write_failed: bool,
}

impl LocalWalJournal {
    pub(crate) fn open(directory: impl AsRef<Path>) -> Result<Self, JournalError> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory).map_err(io_error)?;
        let wal_path = directory.join("meta.wal");
        let snapshot_path = directory.join("meta.snapshot");
        let snapshot = load_snapshot_file(&snapshot_path)?;
        let replay_after = snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.last_applied_index);
        let mut entries = load_wal_file(&wal_path)?;
        entries.retain(|entry| entry.sequence > replay_after);
        let last = entries
            .last()
            .map_or(replay_after, |entry| entry.sequence)
            .max(
                snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.last_applied_index),
            );
        Ok(Self {
            wal_path,
            snapshot_path,
            entries,
            snapshot,
            next_sequence: last + 1,
            write_failed: false,
        })
    }
}

impl MetadataJournal for LocalWalJournal {
    fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
        self.ensure_writable()?;
        let sequence = self.next_sequence;
        let payload = encode_record(&record)?;
        if let Err(error) = append_frame(&self.wal_path, WAL_MAGIC, sequence, &payload) {
            self.write_failed = true;
            return Err(error);
        }
        self.next_sequence += 1;
        self.entries.push(JournalEntry { sequence, record });
        Ok(sequence)
    }

    fn load_after(&self, sequence: u64) -> Result<Vec<JournalEntry>, JournalError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence > sequence)
            .cloned()
            .collect())
    }

    fn load_snapshot(&self) -> Result<Option<MetaSnapshot>, JournalError> {
        Ok(self.snapshot.clone())
    }

    fn save_snapshot(&mut self, snapshot: MetaSnapshot) -> Result<(), JournalError> {
        self.ensure_writable()?;
        if snapshot.last_applied_index
            < self
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.last_applied_index)
        {
            return Err(JournalError::InvalidSnapshot(
                "snapshot index must not move backwards",
            ));
        }
        let payload = encode_snapshot(&snapshot)?;
        if let Err(error) =
            write_snapshot_atomic(&self.snapshot_path, snapshot.last_applied_index, &payload)
        {
            self.write_failed = true;
            return Err(error);
        }
        self.next_sequence = self.next_sequence.max(snapshot.last_applied_index + 1);
        self.snapshot = Some(snapshot);
        Ok(())
    }

    fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError> {
        self.ensure_writable()?;
        // 先落盘，后更新内存；失败时不能让两者保留的日志范围分叉。
        let retained: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.sequence > sequence)
            .cloned()
            .collect();
        if let Err(error) = rewrite_wal(&self.wal_path, &retained) {
            self.write_failed = true;
            return Err(error);
        }
        self.entries = retained;
        Ok(())
    }

    fn last_index(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }
}

impl LocalWalJournal {
    fn ensure_writable(&self) -> Result<(), JournalError> {
        if self.write_failed {
            return Err(JournalError::Unavailable(
                "local WAL write failed; reopen before further mutation",
            ));
        }
        Ok(())
    }
}

fn append_frame(
    path: &Path,
    magic: &[u8; 4],
    sequence: u64,
    payload: &[u8],
) -> Result<(), JournalError> {
    let is_new = !path.exists();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    write_frame(&mut file, magic, sequence, payload)?;
    file.sync_data().map_err(io_error)?;
    if is_new {
        // 首条记录还依赖新目录项持久化；只 sync 文件内容不足以保证重启后能找到文件。
        sync_parent(path)?;
    }
    Ok(())
}

fn rewrite_wal(path: &Path, entries: &[JournalEntry]) -> Result<(), JournalError> {
    let tmp = path.with_extension("wal.tmp");
    {
        let mut file = File::create(&tmp).map_err(io_error)?;
        for entry in entries {
            write_frame(
                &mut file,
                WAL_MAGIC,
                entry.sequence,
                &encode_record(&entry.record)?,
            )?;
        }
        file.sync_data().map_err(io_error)?;
    }
    fs::rename(&tmp, path).map_err(io_error)?;
    sync_parent(path)
}

fn write_snapshot_atomic(path: &Path, sequence: u64, payload: &[u8]) -> Result<(), JournalError> {
    let tmp = path.with_extension("snapshot.tmp");
    {
        let mut file = File::create(&tmp).map_err(io_error)?;
        write_frame(&mut file, SNAPSHOT_MAGIC, sequence, payload)?;
        file.sync_data().map_err(io_error)?;
    }
    fs::rename(&tmp, path).map_err(io_error)?;
    sync_parent(path)
}

fn write_frame(
    file: &mut File,
    magic: &[u8; 4],
    sequence: u64,
    payload: &[u8],
) -> Result<(), JournalError> {
    let length = u32::try_from(payload.len())
        .map_err(|_| JournalError::InvalidSnapshot("journal frame is too large"))?;
    let checksum = checksum(payload);
    file.write_all(magic).map_err(io_error)?;
    file.write_all(&[FRAME_VERSION]).map_err(io_error)?;
    file.write_all(&[0]).map_err(io_error)?;
    file.write_all(&sequence.to_be_bytes()).map_err(io_error)?;
    file.write_all(&length.to_be_bytes()).map_err(io_error)?;
    file.write_all(&checksum.to_be_bytes()).map_err(io_error)?;
    file.write_all(payload).map_err(io_error)?;
    Ok(())
}

/// 解析结果同时携带物理有效边界，避免恢复读取与后续追加使用两套尾部判断。
struct ParsedFrames {
    frames: Vec<(u64, Vec<u8>)>,
    valid_length: u64,
    torn_tail: bool,
}

fn read_frames(path: &Path, magic: &[u8; 4]) -> Result<ParsedFrames, JournalError> {
    if !path.exists() {
        return Ok(ParsedFrames {
            frames: Vec::new(),
            valid_length: 0,
            torn_tail: false,
        });
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let mut offset = 0;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        if bytes.len() - offset < HEADER_LEN {
            break;
        }
        let header = &bytes[offset..offset + HEADER_LEN];
        if &header[0..4] != magic || header[4] != FRAME_VERSION {
            return Err(JournalError::InvalidSnapshot(
                "invalid journal frame header",
            ));
        }
        let sequence = u64::from_be_bytes(copy_array(&header[6..14])?);
        let length = u32::from_be_bytes(copy_array(&header[14..18])?) as usize;
        let expected_checksum = u32::from_be_bytes(copy_array(&header[18..22])?);
        let payload_start = offset + HEADER_LEN;
        let payload_end = payload_start.saturating_add(length);
        if payload_end > bytes.len() {
            break;
        }
        let payload = bytes[payload_start..payload_end].to_vec();
        if checksum(&payload) != expected_checksum {
            return Err(JournalError::InvalidSnapshot(
                "journal frame checksum mismatch",
            ));
        }
        frames.push((sequence, payload));
        offset = payload_end;
    }
    Ok(ParsedFrames {
        frames,
        valid_length: offset as u64,
        torn_tail: offset != bytes.len(),
    })
}

fn load_wal_file(path: &Path) -> Result<Vec<JournalEntry>, JournalError> {
    let ParsedFrames {
        frames,
        valid_length,
        torn_tail,
    } = read_frames(path, WAL_MAGIC)?;
    // 先解码所有完整记录；未知格式/校验损坏不是可丢弃的崩溃残尾。
    let entries = frames
        .into_iter()
        .map(|(sequence, payload)| {
            Ok(JournalEntry {
                sequence,
                record: decode_record(&payload)?,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    if torn_tail {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(io_error)?;
        file.set_len(valid_length).map_err(io_error)?;
        file.sync_data().map_err(io_error)?;
    }
    Ok(entries)
}

fn load_snapshot_file(path: &Path) -> Result<Option<MetaSnapshot>, JournalError> {
    let ParsedFrames {
        mut frames,
        torn_tail,
        ..
    } = read_frames(path, SNAPSHOT_MAGIC)?;
    // snapshot 通过临时文件 + rename 提交；正式文件不允许像 WAL 一样忽略残尾。
    if torn_tail || (path.exists() && frames.len() != 1) {
        return Err(JournalError::InvalidSnapshot("incomplete snapshot file"));
    }
    let Some((sequence, payload)) = frames.pop() else {
        return Ok(None);
    };
    let snapshot = decode_snapshot(&payload)?;
    if snapshot.last_applied_index != sequence {
        return Err(JournalError::InvalidSnapshot("snapshot index mismatch"));
    }
    Ok(Some(snapshot))
}

fn sync_parent(path: &Path) -> Result<(), JournalError> {
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|file| file.sync_data())
            .map_err(io_error)?;
    }
    Ok(())
}

fn checksum(payload: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(payload);
    hasher.finalize()
}

fn encode_record(record: &JournalRecord) -> Result<Vec<u8>, JournalError> {
    let mut out = Vec::new();
    match record {
        JournalRecord::NodeSessionOpened {
            node_id,
            node_epoch,
            session_id,
            control_endpoint,
            next_session,
            supports_commit_sequence,
            commit_sequence_floor,
        } => {
            put_u8(&mut out, RECORD_NODE_SESSION_OPENED);
            put_u64(&mut out, *node_id);
            put_u64(&mut out, *node_epoch);
            put_bytes(&mut out, session_id)?;
            put_bytes(&mut out, control_endpoint.as_bytes())?;
            put_u64(&mut out, *next_session);
            put_bool(&mut out, *supports_commit_sequence);
            put_u64(&mut out, *commit_sequence_floor);
        }
        JournalRecord::ReplicaAccepted {
            location,
            length,
            catalog_revision,
        } => {
            put_u8(&mut out, RECORD_REPLICA_ACCEPTED);
            put_message(&mut out, location)?;
            put_u64(&mut out, *length);
            put_u64(&mut out, *catalog_revision);
        }
        JournalRecord::ReplicasReported {
            operation_id,
            accepted,
            rejected_block_ids,
            catalog_revision,
            desired_copies,
        } => {
            put_u8(&mut out, RECORD_REPLICAS_REPORTED);
            put_bytes(&mut out, operation_id)?;
            put_u64(&mut out, *catalog_revision);
            put_u32(&mut out, len_u32(accepted.len())?);
            for (location, length) in accepted {
                put_message(&mut out, location)?;
                put_u64(&mut out, *length);
            }
            put_u32(&mut out, len_u32(rejected_block_ids.len())?);
            for block_id in rejected_block_ids {
                put_bytes(&mut out, block_id)?;
            }
            put_u32(&mut out, *desired_copies);
        }
        JournalRecord::VersionCommitted {
            key,
            layout,
            modified_time_unix_millis,
            new_replicas,
            operation_id,
            operation_digest,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_VERSION_COMMITTED_V2);
            put_bytes(&mut out, key)?;
            put_message(&mut out, layout)?;
            put_i64(&mut out, *modified_time_unix_millis);
            put_u32(&mut out, len_u32(new_replicas.len())?);
            for (location, length) in new_replicas {
                put_message(&mut out, location)?;
                put_u64(&mut out, *length);
            }
            put_bytes(&mut out, operation_id)?;
            put_bytes(&mut out, operation_digest)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::VersionsCommitted {
            commits,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_VERSIONS_COMMITTED_V2);
            put_u32(&mut out, len_u32(commits.len())?);
            for commit in commits {
                put_bytes(&mut out, &commit.key)?;
                put_message(&mut out, &commit.layout)?;
                put_i64(&mut out, commit.modified_time_unix_millis);
                put_u32(&mut out, len_u32(commit.new_replicas.len())?);
                for (location, length) in &commit.new_replicas {
                    put_message(&mut out, location)?;
                    put_u64(&mut out, *length);
                }
                put_bytes(&mut out, &commit.operation_id)?;
                put_bytes(&mut out, &commit.operation_digest)?;
            }
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::OperationRemembered {
            operation_id,
            operation_digest,
            result,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_OPERATION_REMEMBERED);
            put_bytes(&mut out, operation_id)?;
            put_bytes(&mut out, operation_digest)?;
            put_message(&mut out, result)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::NodeEventAcknowledged { node_id, cursor } => {
            put_u8(&mut out, RECORD_NODE_EVENT_ACKNOWLEDGED);
            put_u64(&mut out, *node_id);
            put_u64(&mut out, *cursor);
        }
        JournalRecord::BlockRetirementPrepared { record } => {
            put_u8(&mut out, RECORD_BLOCK_RETIREMENT_PREPARED);
            put_retirement_record(&mut out, record)?;
        }
        JournalRecord::BlockRetirementAcknowledged {
            retirement_id,
            participant,
            ack_kind,
            stage_epoch,
        } => {
            put_u8(&mut out, RECORD_BLOCK_RETIREMENT_ACKNOWLEDGED);
            put_bytes(&mut out, retirement_id)?;
            put_participant(&mut out, participant);
            put_i32(&mut out, *ack_kind as i32);
            put_u64(&mut out, *stage_epoch);
        }
        JournalRecord::BlockRetirementFinalized {
            retirement_id,
            stage_epoch,
        } => {
            put_u8(&mut out, RECORD_BLOCK_RETIREMENT_FINALIZED);
            put_bytes(&mut out, retirement_id)?;
            put_u64(&mut out, *stage_epoch);
        }
        JournalRecord::BlockRetirementReleased {
            retirement_id,
            block_ids,
            stage_epoch,
        } => {
            put_u8(&mut out, RECORD_BLOCK_RETIREMENT_RELEASED);
            put_bytes(&mut out, retirement_id)?;
            put_u32(&mut out, len_u32(block_ids.len())?);
            for block_id in block_ids {
                put_bytes(&mut out, block_id)?;
            }
            put_u64(&mut out, *stage_epoch);
        }
        JournalRecord::FilesystemInodeCreated { record } => {
            put_u8(&mut out, RECORD_FILESYSTEM_INODE_CREATED);
            put_message(&mut out, &inode_to_proto(&record.inode))?;
            put_message(&mut out, &dentry_to_proto(&record.dentry))?;
            put_u64(&mut out, record.next_inode);
            put_u64(&mut out, record.grant_generation);
        }
        JournalRecord::FilesystemVersionCommitted {
            record,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_FILESYSTEM_VERSION_COMMITTED_V2);
            put_version_commit(&mut out, &record.object)?;
            put_message(&mut out, &inode_to_proto(&record.inode))?;
            put_u64(&mut out, record.revoked_grant_generation);
            put_u64(&mut out, record.new_grant_generation);
            put_xattr_updates(&mut out, &record.xattr_updates)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::FilesystemAttributesUpdated {
            record,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_FILESYSTEM_ATTRIBUTES_UPDATED_V2);
            put_bytes(&mut out, &record.operation_id)?;
            put_bytes(&mut out, &record.operation_digest)?;
            put_message(&mut out, &inode_to_proto(&record.inode))?;
            put_u64(&mut out, record.revoked_grant_generation);
            put_u64(&mut out, record.new_grant_generation);
            put_xattr_updates(&mut out, &record.xattr_updates)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::FilesystemXattrUpdated {
            record,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_FILESYSTEM_XATTR_UPDATED);
            put_bytes(&mut out, &record.operation_id)?;
            put_bytes(&mut out, &record.operation_digest)?;
            put_message(&mut out, &inode_to_proto(&record.inode))?;
            put_u64(&mut out, record.revoked_grant_generation);
            put_u64(&mut out, record.new_grant_generation);
            put_xattr_update(&mut out, &record.update)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::FilesystemNamespaceMutated {
            record,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_FILESYSTEM_NAMESPACE_MUTATED_V2);
            put_filesystem_namespace_mutation(&mut out, record)?;
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::FilesystemSymlinkCreated {
            record,
            commit_sequence,
        } => {
            put_u8(&mut out, RECORD_FILESYSTEM_SYMLINK_CREATED_V2);
            put_filesystem_namespace_mutation(&mut out, &record.namespace)?;
            put_version_commit(&mut out, &record.version)?;
            put_u64(&mut out, record.new_grant_generation);
            put_optional_commit_sequence(&mut out, commit_sequence)?;
        }
        JournalRecord::FilesystemOrphanReaped { record } => {
            put_u8(&mut out, RECORD_FILESYSTEM_ORPHAN_REAPED);
            put_u64(&mut out, record.inode);
            put_bool(&mut out, record.object_tombstone.is_some());
            if let Some(tombstone) = &record.object_tombstone {
                put_version_commit(&mut out, tombstone)?;
            }
        }
    }
    Ok(out)
}

fn decode_record(bytes: &[u8]) -> Result<JournalRecord, JournalError> {
    let mut input = Cursor::new(bytes);
    let kind = input.u8()?;
    match kind {
        RECORD_NODE_SESSION_OPENED => Ok(JournalRecord::NodeSessionOpened {
            node_id: input.u64()?,
            node_epoch: input.u64()?,
            session_id: input.bytes()?,
            control_endpoint: String::from_utf8(input.bytes()?)
                .map_err(|_| JournalError::InvalidSnapshot("invalid session endpoint"))?,
            next_session: input.u64()?,
            supports_commit_sequence: input.remaining() > 0 && input.bool()?,
            commit_sequence_floor: if input.remaining() >= 8 {
                input.u64()?
            } else {
                0
            },
        }),
        RECORD_REPLICA_ACCEPTED => Ok(JournalRecord::ReplicaAccepted {
            location: input.message()?,
            length: input.u64()?,
            catalog_revision: input.u64()?,
        }),
        RECORD_REPLICAS_REPORTED => {
            let operation_id = input.bytes()?;
            let catalog_revision = input.u64()?;
            let accepted = (0..input.u32()?)
                .map(|_| Ok((input.message()?, input.u64()?)))
                .collect::<Result<Vec<_>, JournalError>>()?;
            let rejected_block_ids = (0..input.u32()?)
                .map(|_| input.bytes())
                .collect::<Result<Vec<_>, JournalError>>()?;
            // Backward-compatible decode for journals written before explicit
            // replica policy was added.
            let desired_copies = if input.remaining() >= 4 {
                input.u32()?
            } else {
                1
            };
            Ok(JournalRecord::ReplicasReported {
                operation_id,
                accepted,
                rejected_block_ids,
                catalog_revision,
                desired_copies,
            })
        }
        RECORD_VERSION_COMMITTED | RECORD_VERSION_COMMITTED_V2 => {
            let key = input.bytes()?;
            let layout = input.message()?;
            let modified_time_unix_millis = if kind == RECORD_VERSION_COMMITTED_V2 {
                input.i64()?
            } else {
                0
            };
            let new_replicas = (0..input.u32()?)
                .map(|_| Ok((input.message()?, input.u64()?)))
                .collect::<Result<Vec<_>, JournalError>>()?;
            Ok(JournalRecord::VersionCommitted {
                key,
                layout,
                modified_time_unix_millis,
                new_replicas,
                operation_id: input.bytes()?,
                operation_digest: input.bytes()?,
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_VERSIONS_COMMITTED | RECORD_VERSIONS_COMMITTED_V2 => {
            let commits = (0..input.u32()?)
                .map(|_| {
                    let key = input.bytes()?;
                    let layout = input.message()?;
                    let modified_time_unix_millis = if kind == RECORD_VERSIONS_COMMITTED_V2 {
                        input.i64()?
                    } else {
                        0
                    };
                    let new_replicas = (0..input.u32()?)
                        .map(|_| Ok((input.message()?, input.u64()?)))
                        .collect::<Result<Vec<_>, JournalError>>()?;
                    Ok(super::metadata_journal::VersionCommitRecord {
                        key,
                        layout,
                        modified_time_unix_millis,
                        new_replicas,
                        operation_id: input.bytes()?,
                        operation_digest: input.bytes()?,
                    })
                })
                .collect::<Result<Vec<_>, JournalError>>()?;
            Ok(JournalRecord::VersionsCommitted {
                commits,
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_OPERATION_REMEMBERED => Ok(JournalRecord::OperationRemembered {
            operation_id: input.bytes()?,
            operation_digest: input.bytes()?,
            result: input.message()?,
            commit_sequence: input.optional_commit_sequence()?,
        }),
        RECORD_NODE_EVENT_ACKNOWLEDGED => Ok(JournalRecord::NodeEventAcknowledged {
            node_id: input.u64()?,
            cursor: input.u64()?,
        }),
        RECORD_BLOCK_RETIREMENT_PREPARED => Ok(JournalRecord::BlockRetirementPrepared {
            record: input.retirement_record()?,
        }),
        RECORD_BLOCK_RETIREMENT_ACKNOWLEDGED => Ok(JournalRecord::BlockRetirementAcknowledged {
            retirement_id: input.bytes()?,
            participant: input.participant()?,
            ack_kind: pb::BlockRetirementAckKind::try_from(input.i32()?)
                .map_err(|_| JournalError::InvalidSnapshot("invalid retirement ack kind"))?,
            stage_epoch: input.u64()?,
        }),
        RECORD_BLOCK_RETIREMENT_FINALIZED => Ok(JournalRecord::BlockRetirementFinalized {
            retirement_id: input.bytes()?,
            stage_epoch: input.u64()?,
        }),
        RECORD_BLOCK_RETIREMENT_RELEASED => {
            let retirement_id = input.bytes()?;
            let block_ids = (0..input.u32()?)
                .map(|_| input.bytes())
                .collect::<Result<Vec<_>, JournalError>>()?;
            Ok(JournalRecord::BlockRetirementReleased {
                retirement_id,
                block_ids,
                stage_epoch: input.u64()?,
            })
        }
        RECORD_FILESYSTEM_INODE_CREATED => Ok(JournalRecord::FilesystemInodeCreated {
            record: FilesystemInodeCreatedRecord {
                inode: decode_inode(input.message()?)?,
                dentry: dentry_from_proto(input.message()?),
                next_inode: input.u64()?,
                grant_generation: input.u64()?,
            },
        }),
        RECORD_FILESYSTEM_VERSION_COMMITTED | RECORD_FILESYSTEM_VERSION_COMMITTED_V2 => {
            let object = input.version_commit()?;
            let inode = decode_inode(input.message()?)?;
            let revoked_grant_generation = input.u64()?;
            let new_grant_generation = input.u64()?;
            let xattr_updates = if kind == RECORD_FILESYSTEM_VERSION_COMMITTED_V2 {
                input.xattr_updates()?
            } else {
                Vec::new()
            };
            Ok(JournalRecord::FilesystemVersionCommitted {
                record: FilesystemVersionCommitRecord {
                    object,
                    inode,
                    revoked_grant_generation,
                    new_grant_generation,
                    xattr_updates,
                },
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_FILESYSTEM_ATTRIBUTES_UPDATED | RECORD_FILESYSTEM_ATTRIBUTES_UPDATED_V2 => {
            let operation_id = input.bytes()?;
            let operation_digest = input.bytes()?;
            let inode = decode_inode(input.message()?)?;
            let revoked_grant_generation = input.u64()?;
            let new_grant_generation = input.u64()?;
            let xattr_updates = if kind == RECORD_FILESYSTEM_ATTRIBUTES_UPDATED_V2 {
                input.xattr_updates()?
            } else {
                Vec::new()
            };
            Ok(JournalRecord::FilesystemAttributesUpdated {
                record: FilesystemAttributesUpdatedRecord {
                    operation_id,
                    operation_digest,
                    inode,
                    revoked_grant_generation,
                    new_grant_generation,
                    xattr_updates,
                },
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_FILESYSTEM_XATTR_UPDATED => Ok(JournalRecord::FilesystemXattrUpdated {
            record: FilesystemXattrUpdatedRecord {
                operation_id: input.bytes()?,
                operation_digest: input.bytes()?,
                inode: decode_inode(input.message()?)?,
                revoked_grant_generation: input.u64()?,
                new_grant_generation: input.u64()?,
                update: input.xattr_update()?,
            },
            commit_sequence: input.optional_commit_sequence()?,
        }),
        RECORD_FILESYSTEM_NAMESPACE_MUTATED | RECORD_FILESYSTEM_NAMESPACE_MUTATED_V2 => {
            Ok(JournalRecord::FilesystemNamespaceMutated {
                record: input.filesystem_namespace_mutation(
                    kind == RECORD_FILESYSTEM_NAMESPACE_MUTATED_V2,
                )?,
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_FILESYSTEM_SYMLINK_CREATED | RECORD_FILESYSTEM_SYMLINK_CREATED_V2 => {
            Ok(JournalRecord::FilesystemSymlinkCreated {
                record: FilesystemSymlinkCreatedRecord {
                    namespace: input.filesystem_namespace_mutation(
                        kind == RECORD_FILESYSTEM_SYMLINK_CREATED_V2,
                    )?,
                    version: input.version_commit()?,
                    new_grant_generation: input.u64()?,
                },
                commit_sequence: input.optional_commit_sequence()?,
            })
        }
        RECORD_FILESYSTEM_ORPHAN_REAPED => {
            let inode = input.u64()?;
            let object_tombstone = input.bool()?.then(|| input.version_commit()).transpose()?;
            Ok(JournalRecord::FilesystemOrphanReaped {
                record: FilesystemOrphanReapedRecord {
                    inode,
                    object_tombstone,
                },
            })
        }
        _ => Err(JournalError::InvalidSnapshot("unknown journal record kind")),
    }
}

fn encode_snapshot(snapshot: &MetaSnapshot) -> Result<Vec<u8>, JournalError> {
    let mut out = Vec::new();
    put_u64(&mut out, snapshot.last_applied_index);
    put_u64(&mut out, snapshot.next_session);
    put_u64(&mut out, snapshot.event_high_watermark);
    put_u32(&mut out, len_u32(snapshot.node_epochs.len())?);
    for (node_id, epoch) in &snapshot.node_epochs {
        put_u64(&mut out, *node_id);
        put_u64(&mut out, *epoch);
    }
    put_u32(&mut out, len_u32(snapshot.sessions.len())?);
    for session in &snapshot.sessions {
        put_u64(&mut out, session.node_id);
        put_bytes(&mut out, &session.session_id)?;
        put_u64(&mut out, session.node_epoch);
        put_bytes(&mut out, session.control_endpoint.as_bytes())?;
        put_u64(&mut out, session.last_acked_cursor);
    }
    put_u32(&mut out, len_u32(snapshot.replicas.len())?);
    for replica in &snapshot.replicas {
        put_bytes(&mut out, &replica.block_id)?;
        put_message(&mut out, &replica.location)?;
        put_u64(&mut out, replica.catalog_revision);
        put_u64(&mut out, replica.length);
    }
    put_u32(&mut out, len_u32(snapshot.versions.len())?);
    for (key, layouts) in &snapshot.versions {
        put_bytes(&mut out, key)?;
        put_u32(&mut out, len_u32(layouts.len())?);
        for layout in layouts {
            put_message(&mut out, layout)?;
        }
    }
    put_u32(&mut out, len_u32(snapshot.operations.len())?);
    for operation in &snapshot.operations {
        put_bytes(&mut out, &operation.operation_id)?;
        put_bytes(&mut out, &operation.digest)?;
        put_message(&mut out, &operation.result)?;
    }
    put_u32(&mut out, len_u32(snapshot.replica_operations.len())?);
    for operation in &snapshot.replica_operations {
        put_bytes(&mut out, &operation.operation_id)?;
        put_message(&mut out, &operation.result)?;
    }
    put_u32(&mut out, len_u32(snapshot.events.len())?);
    for event in &snapshot.events {
        put_message(&mut out, event)?;
    }
    put_u32(&mut out, len_u32(snapshot.desired_replica_counts.len())?);
    for (block_id, copies) in &snapshot.desired_replica_counts {
        put_bytes(&mut out, block_id)?;
        put_u32(&mut out, *copies);
    }
    put_u32(&mut out, len_u32(snapshot.version_modified_times.len())?);
    for (key, version, modified_time) in &snapshot.version_modified_times {
        put_bytes(&mut out, key)?;
        put_u64(&mut out, *version);
        put_i64(&mut out, *modified_time);
    }
    put_u32(&mut out, len_u32(snapshot.block_retirements.len())?);
    for retirement in &snapshot.block_retirements {
        put_retirement_record(&mut out, &retirement.record)?;
        put_u32(&mut out, len_u32(retirement.prepared.len())?);
        for participant in &retirement.prepared {
            put_participant(&mut out, participant);
        }
        put_u32(&mut out, len_u32(retirement.released.len())?);
        for participant in &retirement.released {
            put_participant(&mut out, participant);
        }
        put_bool(&mut out, retirement.final_sent);
    }
    put_u32(&mut out, len_u32(snapshot.retired_block_fences.len())?);
    for (block_id, fence_version) in &snapshot.retired_block_fences {
        put_bytes(&mut out, block_id)?;
        put_u64(&mut out, *fence_version);
    }
    put_u64(&mut out, snapshot.version_floor);
    put_u32(
        &mut out,
        len_u32(snapshot.node_commit_sequence_floors.len())?,
    );
    for (node_id, floor) in &snapshot.node_commit_sequence_floors {
        put_u64(&mut out, *node_id);
        put_u64(&mut out, *floor);
    }
    let session_commit_states = snapshot
        .sessions
        .iter()
        .filter(|session| session.supports_commit_sequence || session.commit_sequence_floor > 0)
        .collect::<Vec<_>>();
    put_u32(&mut out, len_u32(session_commit_states.len())?);
    for session in session_commit_states {
        put_u64(&mut out, session.node_id);
        put_u64(&mut out, session.node_epoch);
        put_bool(&mut out, session.supports_commit_sequence);
        put_u64(&mut out, session.commit_sequence_floor);
    }
    put_u32(&mut out, len_u32(snapshot.commit_sequences.len())?);
    for record in &snapshot.commit_sequences {
        put_commit_sequence(&mut out, record)?;
    }
    // Filesystem fields are appended so snapshots produced before this feature remain readable.
    put_u64(&mut out, snapshot.filesystem_next_inode);
    put_u32(&mut out, len_u32(snapshot.filesystem_inodes.len())?);
    for inode in &snapshot.filesystem_inodes {
        put_message(&mut out, &inode_to_proto(inode))?;
    }
    put_u32(&mut out, len_u32(snapshot.filesystem_dentries.len())?);
    for dentry in &snapshot.filesystem_dentries {
        put_message(&mut out, &dentry_to_proto(dentry))?;
    }
    put_u32(
        &mut out,
        len_u32(snapshot.filesystem_grant_generations.len())?,
    );
    for (inode, generation) in &snapshot.filesystem_grant_generations {
        put_u64(&mut out, *inode);
        put_u64(&mut out, *generation);
    }
    put_u32(
        &mut out,
        len_u32(snapshot.filesystem_namespace_operations.len())?,
    );
    for operation in &snapshot.filesystem_namespace_operations {
        put_bytes(&mut out, &operation.operation_id)?;
        put_bytes(&mut out, &operation.digest)?;
        put_message(&mut out, &namespace_result_to_proto(&operation.result))?;
    }
    put_u32(
        &mut out,
        len_u32(snapshot.filesystem_version_operations.len())?,
    );
    for operation in &snapshot.filesystem_version_operations {
        put_bytes(&mut out, &operation.operation_id)?;
        put_bytes(&mut out, &operation.digest)?;
        put_message(&mut out, &operation.response)?;
    }
    let visibility = snapshot
        .filesystem_operation_visibility
        .as_ref()
        .expect("new snapshots must declare filesystem operation visibility");
    put_u32(&mut out, len_u32(visibility.namespace.len())?);
    for (operation_id, cursor) in &visibility.namespace {
        put_bytes(&mut out, operation_id)?;
        put_u64(&mut out, *cursor);
    }
    put_u32(&mut out, len_u32(visibility.versions.len())?);
    for (operation_id, cursor) in &visibility.versions {
        put_bytes(&mut out, operation_id)?;
        put_u64(&mut out, *cursor);
    }
    // M1.4 作为 snapshot 尾部扩展追加；旧 snapshot 在这里自然 EOF，新 decoder
    // 使用空集合恢复，避免改写前面已经发布的二进制布局。
    put_u32(
        &mut out,
        len_u32(snapshot.filesystem_attribute_operations.len())?,
    );
    for operation in &snapshot.filesystem_attribute_operations {
        put_bytes(&mut out, &operation.operation_id)?;
        put_bytes(&mut out, &operation.digest)?;
        put_message(&mut out, &operation.response)?;
    }
    put_u32(&mut out, len_u32(visibility.attributes.len())?);
    for (operation_id, cursor) in &visibility.attributes {
        put_bytes(&mut out, operation_id)?;
        put_u64(&mut out, *cursor);
    }
    put_u32(&mut out, len_u32(snapshot.filesystem_xattrs.len())?);
    for (inode, name, value) in &snapshot.filesystem_xattrs {
        put_u64(&mut out, *inode);
        put_bytes(&mut out, name)?;
        put_bytes(&mut out, value)?;
    }
    Ok(out)
}

fn decode_snapshot(bytes: &[u8]) -> Result<MetaSnapshot, JournalError> {
    let mut input = Cursor::new(bytes);
    let last_applied_index = input.u64()?;
    let next_session = input.u64()?;
    let event_high_watermark = input.u64()?;
    let node_epochs = (0..input.u32()?)
        .map(|_| Ok((input.u64()?, input.u64()?)))
        .collect::<Result<Vec<_>, JournalError>>()?;
    let mut sessions = (0..input.u32()?)
        .map(|_| {
            Ok(SnapshotSession {
                node_id: input.u64()?,
                session_id: input.bytes()?,
                node_epoch: input.u64()?,
                control_endpoint: String::from_utf8(input.bytes()?)
                    .map_err(|_| JournalError::InvalidSnapshot("invalid snapshot endpoint"))?,
                last_acked_cursor: input.u64()?,
                supports_commit_sequence: false,
                commit_sequence_floor: 0,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let replicas = (0..input.u32()?)
        .map(|_| {
            Ok(SnapshotReplica {
                block_id: input.bytes()?,
                location: input.message()?,
                catalog_revision: input.u64()?,
                length: input.u64()?,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let versions = (0..input.u32()?)
        .map(|_| {
            let key = input.bytes()?;
            let layouts = (0..input.u32()?)
                .map(|_| input.message::<pb::VersionLayout>())
                .collect::<Result<Vec<_>, JournalError>>()?;
            Ok((key, layouts))
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let operations = (0..input.u32()?)
        .map(|_| {
            Ok(SnapshotOperation {
                operation_id: input.bytes()?,
                digest: input.bytes()?,
                result: input.message()?,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let replica_operations = (0..input.u32()?)
        .map(|_| {
            Ok(SnapshotReplicaOperation {
                operation_id: input.bytes()?,
                result: input.message()?,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let events = (0..input.u32()?)
        .map(|_| input.message())
        .collect::<Result<Vec<_>, JournalError>>()?;
    let desired_replica_counts = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u32()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let version_modified_times = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u64()?, input.i64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let block_retirements = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| {
                let record = input.retirement_record()?;
                let prepared = (0..input.u32()?)
                    .map(|_| input.participant())
                    .collect::<Result<Vec<_>, JournalError>>()?;
                let released = (0..input.u32()?)
                    .map(|_| input.participant())
                    .collect::<Result<Vec<_>, JournalError>>()?;
                Ok(SnapshotBlockRetirement {
                    record,
                    prepared,
                    released,
                    final_sent: input.bool()?,
                })
            })
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let retired_block_fences = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let version_floor = if input.remaining() == 0 {
        versions
            .iter()
            .flat_map(|(_, layouts)| layouts.iter().map(|layout| layout.version))
            .max()
            .unwrap_or(0)
    } else {
        input.u64()?
    };
    let node_commit_sequence_floors = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.u64()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    if input.remaining() > 0 {
        for _ in 0..input.u32()? {
            let node_id = input.u64()?;
            let node_epoch = input.u64()?;
            let supports_commit_sequence = input.bool()?;
            let commit_sequence_floor = input.u64()?;
            if let Some(session) = sessions
                .iter_mut()
                .find(|session| session.node_id == node_id && session.node_epoch == node_epoch)
            {
                session.supports_commit_sequence = supports_commit_sequence;
                session.commit_sequence_floor = commit_sequence_floor;
            }
        }
    }
    let commit_sequences = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| input.commit_sequence())
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_next_inode = if input.remaining() == 0 {
        2
    } else {
        input.u64()?.max(2)
    };
    let filesystem_inodes = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| decode_inode(input.message()?))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_dentries = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok(dentry_from_proto(input.message()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_grant_generations = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.u64()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_namespace_operations = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| {
                Ok(SnapshotFilesystemNamespaceOperation {
                    operation_id: input.bytes()?,
                    digest: input.bytes()?,
                    result: namespace_result_from_proto(input.message()?).map_err(|_| {
                        JournalError::InvalidSnapshot("invalid filesystem namespace operation")
                    })?,
                })
            })
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_version_operations = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| {
                Ok(SnapshotFilesystemVersionOperation {
                    operation_id: input.bytes()?,
                    digest: input.bytes()?,
                    response: input.message()?,
                })
            })
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let mut filesystem_operation_visibility = if input.remaining() == 0 {
        None
    } else {
        let namespace = (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?;
        let versions = (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?;
        Some(SnapshotFilesystemOperationVisibility {
            namespace,
            versions,
            attributes: Vec::new(),
        })
    };
    let filesystem_attribute_operations = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| {
                Ok(SnapshotFilesystemAttributeOperation {
                    operation_id: input.bytes()?,
                    digest: input.bytes()?,
                    response: input.message()?,
                })
            })
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    let filesystem_attribute_visibility = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.bytes()?, input.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    if let Some(visibility) = filesystem_operation_visibility.as_mut() {
        visibility.attributes = filesystem_attribute_visibility;
    }
    let filesystem_xattrs = if input.remaining() == 0 {
        Vec::new()
    } else {
        (0..input.u32()?)
            .map(|_| Ok((input.u64()?, input.bytes()?, input.bytes()?)))
            .collect::<Result<Vec<_>, JournalError>>()?
    };
    if input.remaining() != 0 {
        return Err(JournalError::InvalidSnapshot(
            "unexpected trailing filesystem snapshot bytes",
        ));
    }
    Ok(MetaSnapshot {
        last_applied_index,
        version_floor,
        next_session,
        node_epochs,
        node_commit_sequence_floors,
        sessions,
        replicas,
        desired_replica_counts,
        versions,
        version_modified_times,
        block_retirements,
        retired_block_fences,
        commit_sequences,
        operations,
        replica_operations,
        event_high_watermark,
        events,
        filesystem_next_inode,
        filesystem_inodes,
        filesystem_dentries,
        filesystem_grant_generations,
        filesystem_xattrs,
        filesystem_namespace_operations,
        filesystem_version_operations,
        filesystem_attribute_operations,
        filesystem_operation_visibility,
    })
}

fn decode_inode(
    inode: pb::FilesystemInodeSnapshot,
) -> Result<crate::filesystem::InodeSnapshot, JournalError> {
    inode_from_proto(inode)
        .map_err(|_| JournalError::InvalidSnapshot("invalid filesystem inode snapshot"))
}

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_bool(out: &mut Vec<u8>, value: bool) {
    put_u8(out, u8::from(value));
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8]) -> Result<(), JournalError> {
    put_u32(out, len_u32(value.len())?);
    out.extend_from_slice(value);
    Ok(())
}

fn put_participant(out: &mut Vec<u8>, participant: &BlockRetirementParticipant) {
    put_u64(out, participant.node_id);
    put_u64(out, participant.node_epoch);
}

fn put_optional_commit_sequence(
    out: &mut Vec<u8>,
    record: &Option<CommitSequenceRecord>,
) -> Result<(), JournalError> {
    put_bool(out, record.is_some());
    if let Some(record) = record {
        put_commit_sequence(out, record)?;
    }
    Ok(())
}

fn put_commit_sequence(
    out: &mut Vec<u8>,
    record: &CommitSequenceRecord,
) -> Result<(), JournalError> {
    put_u64(out, record.node_id);
    put_u64(out, record.node_epoch);
    put_u64(out, record.commit_sequence);
    put_bytes(out, &record.operation_id)?;
    put_bytes(out, &record.operation_digest)?;
    put_u64(out, record.commit_index);
    Ok(())
}

fn put_version_commit(out: &mut Vec<u8>, commit: &VersionCommitRecord) -> Result<(), JournalError> {
    put_bytes(out, &commit.key)?;
    put_message(out, &commit.layout)?;
    put_i64(out, commit.modified_time_unix_millis);
    put_u32(out, len_u32(commit.new_replicas.len())?);
    for (location, length) in &commit.new_replicas {
        put_message(out, location)?;
        put_u64(out, *length);
    }
    put_bytes(out, &commit.operation_id)?;
    put_bytes(out, &commit.operation_digest)
}

fn put_filesystem_namespace_mutation(
    out: &mut Vec<u8>,
    record: &FilesystemNamespaceMutationRecord,
) -> Result<(), JournalError> {
    put_bytes(out, &record.operation_id)?;
    put_bytes(out, &record.operation_digest)?;
    put_message(out, &namespace_result_to_proto(&record.result))?;
    put_u32(out, len_u32(record.upsert_inodes.len())?);
    for inode in &record.upsert_inodes {
        put_message(out, &inode_to_proto(inode))?;
    }
    put_u32(out, len_u32(record.upsert_dentries.len())?);
    for dentry in &record.upsert_dentries {
        put_message(out, &dentry_to_proto(dentry))?;
    }
    put_u32(out, len_u32(record.remove_dentries.len())?);
    for dentry in &record.remove_dentries {
        put_message(out, &dentry_to_proto(dentry))?;
    }
    put_u64(out, record.next_inode);
    put_xattr_updates(out, &record.xattr_updates)?;
    Ok(())
}

fn put_xattr_update(out: &mut Vec<u8>, update: &XattrUpdate) -> Result<(), JournalError> {
    put_u64(out, update.inode);
    put_bytes(out, &update.name)?;
    put_bool(out, update.value.is_some());
    if let Some(value) = &update.value {
        put_bytes(out, value)?;
    }
    Ok(())
}

fn put_xattr_updates(out: &mut Vec<u8>, updates: &[XattrUpdate]) -> Result<(), JournalError> {
    put_u32(out, len_u32(updates.len())?);
    for update in updates {
        put_xattr_update(out, update)?;
    }
    Ok(())
}

fn put_retirement_record(
    out: &mut Vec<u8>,
    record: &BlockRetirementRecord,
) -> Result<(), JournalError> {
    put_bytes(out, &record.retirement_id)?;
    put_u32(out, len_u32(record.block_ids.len())?);
    for block_id in &record.block_ids {
        put_bytes(out, block_id)?;
    }
    put_u32(out, len_u32(record.participants.len())?);
    for participant in &record.participants {
        put_participant(out, participant);
    }
    put_u64(out, record.prepare_stage_epoch);
    put_u64(out, record.final_stage_epoch);
    put_u64(out, record.fence_version);
    Ok(())
}

fn put_message<M: Message>(out: &mut Vec<u8>, message: &M) -> Result<(), JournalError> {
    let mut bytes = Vec::new();
    message
        .encode(&mut bytes)
        .map_err(|_| JournalError::InvalidSnapshot("failed to encode protobuf message"))?;
    put_bytes(out, &bytes)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u8(&mut self) -> Result<u8, JournalError> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or(JournalError::InvalidSnapshot("truncated u8"))?;
        self.offset += 1;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, JournalError> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes(copy_array(bytes)?))
    }

    fn u64(&mut self) -> Result<u64, JournalError> {
        let bytes = self.take(8)?;
        Ok(u64::from_be_bytes(copy_array(bytes)?))
    }

    fn i32(&mut self) -> Result<i32, JournalError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes(copy_array(bytes)?))
    }

    fn i64(&mut self) -> Result<i64, JournalError> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes(copy_array(bytes)?))
    }

    fn bool(&mut self) -> Result<bool, JournalError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(JournalError::InvalidSnapshot("invalid bool")),
        }
    }

    fn bytes(&mut self) -> Result<Vec<u8>, JournalError> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }

    fn message<M: Message + Default>(&mut self) -> Result<M, JournalError> {
        M::decode(self.bytes()?.as_slice())
            .map_err(|_| JournalError::InvalidSnapshot("failed to decode protobuf message"))
    }

    fn participant(&mut self) -> Result<BlockRetirementParticipant, JournalError> {
        Ok(BlockRetirementParticipant {
            node_id: self.u64()?,
            node_epoch: self.u64()?,
        })
    }

    fn optional_commit_sequence(&mut self) -> Result<Option<CommitSequenceRecord>, JournalError> {
        if self.remaining() == 0 || !self.bool()? {
            return Ok(None);
        }
        self.commit_sequence().map(Some)
    }

    fn commit_sequence(&mut self) -> Result<CommitSequenceRecord, JournalError> {
        Ok(CommitSequenceRecord {
            node_id: self.u64()?,
            node_epoch: self.u64()?,
            commit_sequence: self.u64()?,
            operation_id: self.bytes()?,
            operation_digest: self.bytes()?,
            commit_index: self.u64()?,
        })
    }

    fn version_commit(&mut self) -> Result<VersionCommitRecord, JournalError> {
        let key = self.bytes()?;
        let layout = self.message()?;
        let modified_time_unix_millis = self.i64()?;
        let new_replicas = (0..self.u32()?)
            .map(|_| Ok((self.message()?, self.u64()?)))
            .collect::<Result<Vec<_>, JournalError>>()?;
        Ok(VersionCommitRecord {
            key,
            layout,
            modified_time_unix_millis,
            new_replicas,
            operation_id: self.bytes()?,
            operation_digest: self.bytes()?,
        })
    }

    fn filesystem_namespace_mutation(
        &mut self,
        has_xattrs: bool,
    ) -> Result<FilesystemNamespaceMutationRecord, JournalError> {
        let operation_id = self.bytes()?;
        let operation_digest = self.bytes()?;
        let result = namespace_result_from_proto(self.message()?)
            .map_err(|_| JournalError::InvalidSnapshot("invalid filesystem namespace result"))?;
        let upsert_inodes = (0..self.u32()?)
            .map(|_| decode_inode(self.message()?))
            .collect::<Result<Vec<_>, JournalError>>()?;
        let upsert_dentries = (0..self.u32()?)
            .map(|_| Ok(dentry_from_proto(self.message()?)))
            .collect::<Result<Vec<_>, JournalError>>()?;
        let remove_dentries = (0..self.u32()?)
            .map(|_| Ok(dentry_from_proto(self.message()?)))
            .collect::<Result<Vec<_>, JournalError>>()?;
        let next_inode = self.u64()?;
        let xattr_updates = if has_xattrs {
            self.xattr_updates()?
        } else {
            Vec::new()
        };
        Ok(FilesystemNamespaceMutationRecord {
            operation_id,
            operation_digest,
            result,
            upsert_inodes,
            upsert_dentries,
            remove_dentries,
            next_inode,
            xattr_updates,
        })
    }

    fn xattr_update(&mut self) -> Result<XattrUpdate, JournalError> {
        Ok(XattrUpdate {
            inode: self.u64()?,
            name: self.bytes()?,
            value: self.bool()?.then(|| self.bytes()).transpose()?,
        })
    }

    fn xattr_updates(&mut self) -> Result<Vec<XattrUpdate>, JournalError> {
        (0..self.u32()?).map(|_| self.xattr_update()).collect()
    }

    fn retirement_record(&mut self) -> Result<BlockRetirementRecord, JournalError> {
        let retirement_id = self.bytes()?;
        let block_ids = (0..self.u32()?)
            .map(|_| self.bytes())
            .collect::<Result<Vec<_>, JournalError>>()?;
        let participants = (0..self.u32()?)
            .map(|_| self.participant())
            .collect::<Result<Vec<_>, JournalError>>()?;
        Ok(BlockRetirementRecord {
            retirement_id,
            block_ids,
            participants,
            prepare_stage_epoch: self.u64()?,
            final_stage_epoch: self.u64()?,
            fence_version: self.u64()?,
        })
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], JournalError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(JournalError::InvalidSnapshot("cursor overflow"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(JournalError::InvalidSnapshot("truncated payload"))?;
        self.offset = end;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}

fn len_u32(length: usize) -> Result<u32, JournalError> {
    u32::try_from(length).map_err(|_| JournalError::InvalidSnapshot("encoded list is too large"))
}

fn copy_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], JournalError> {
    bytes
        .try_into()
        .map_err(|_| JournalError::InvalidSnapshot("invalid integer length"))
}

fn io_error(_error: io::Error) -> JournalError {
    JournalError::Unavailable("local WAL I/O failed")
}

#[cfg(test)]
mod tests {
    use dms_protocol::v1 as pb;

    use super::*;

    #[test]
    fn wal_recovers_snapshot_and_tail_with_monotonic_index() {
        let directory = temp_dir("snapshot-tail");
        let mut journal = LocalWalJournal::open(&directory).expect("open");
        let first = journal
            .append(JournalRecord::OperationRemembered {
                operation_id: b"op-1".to_vec(),
                operation_digest: b"d1".to_vec(),
                result: pb::CommitVersionResponse {
                    version: 0,
                    revision: 1,
                    commit_index: 1,
                    changed: false,
                },
                commit_sequence: None,
            })
            .expect("append");
        journal
            .save_snapshot(MetaSnapshot {
                last_applied_index: first,
                version_floor: 0,
                next_session: 2,
                node_epochs: vec![(7, 1)],
                node_commit_sequence_floors: Vec::new(),
                sessions: Vec::new(),
                replicas: Vec::new(),
                desired_replica_counts: vec![(b"block-a".to_vec(), 2)],
                versions: Vec::new(),
                version_modified_times: Vec::new(),
                block_retirements: Vec::new(),
                retired_block_fences: Vec::new(),
                commit_sequences: Vec::new(),
                operations: Vec::new(),
                replica_operations: Vec::new(),
                event_high_watermark: 0,
                events: Vec::new(),
                filesystem_next_inode: 2,
                filesystem_inodes: Vec::new(),
                filesystem_dentries: Vec::new(),
                filesystem_grant_generations: Vec::new(),
                filesystem_xattrs: Vec::new(),
                filesystem_namespace_operations: Vec::new(),
                filesystem_version_operations: Vec::new(),
                filesystem_attribute_operations: Vec::new(),
                filesystem_operation_visibility: Some(SnapshotFilesystemOperationVisibility {
                    namespace: vec![(b"namespace-op".to_vec(), 7)],
                    versions: vec![(b"version-op".to_vec(), 9)],
                    attributes: Vec::new(),
                }),
            })
            .expect("snapshot");
        journal.truncate_prefix(first).expect("truncate");
        let second = journal
            .append(JournalRecord::NodeEventAcknowledged {
                node_id: 7,
                cursor: 9,
            })
            .expect("append tail");
        drop(journal);

        let restored = LocalWalJournal::open(&directory).expect("restore");
        let snapshot = restored.load_snapshot().expect("snapshot").unwrap();
        assert_eq!(snapshot.next_session, 2);
        assert_eq!(
            snapshot.desired_replica_counts,
            vec![(b"block-a".to_vec(), 2)]
        );
        assert_eq!(
            snapshot.filesystem_operation_visibility,
            Some(SnapshotFilesystemOperationVisibility {
                namespace: vec![(b"namespace-op".to_vec(), 7)],
                versions: vec![(b"version-op".to_vec(), 9)],
                attributes: Vec::new(),
            })
        );
        assert_eq!(restored.load_after(first).expect("tail").len(), 1);
        assert_eq!(restored.last_index(), second);
    }

    #[test]
    fn wal_ignores_torn_tail_but_rejects_corruption() {
        let directory = temp_dir("torn-tail");
        let mut journal = LocalWalJournal::open(&directory).expect("open");
        journal
            .append(JournalRecord::NodeEventAcknowledged {
                node_id: 1,
                cursor: 1,
            })
            .expect("append");
        drop(journal);
        let wal = directory.join("meta.wal");
        OpenOptions::new()
            .append(true)
            .open(&wal)
            .expect("wal")
            .write_all(b"partial")
            .expect("torn");
        assert_eq!(
            LocalWalJournal::open(&directory)
                .expect("restore")
                .load_after(0)
                .expect("entries")
                .len(),
            1
        );

        let bytes = fs::read(&wal).expect("read wal");
        let mut corrupt = bytes;
        corrupt[HEADER_LEN] ^= 0xff;
        fs::write(&wal, corrupt).expect("corrupt wal");
        assert!(LocalWalJournal::open(&directory).is_err());
    }

    /// 不仅“能读旧记录”：修复残尾后必须还能追加，并再次恢复。
    #[test]
    fn wal_repairs_torn_header_and_payload_before_next_append() {
        for partial_payload in [false, true] {
            let directory = temp_dir("repair-and-reopen");
            let mut journal = LocalWalJournal::open(&directory).expect("open");
            journal
                .append(JournalRecord::NodeEventAcknowledged {
                    node_id: 1,
                    cursor: 1,
                })
                .expect("first commit");
            let wal = directory.join("meta.wal");
            let valid_length = fs::metadata(&wal).unwrap().len();
            journal
                .append(JournalRecord::NodeEventAcknowledged {
                    node_id: 1,
                    cursor: 2,
                })
                .expect("second frame before simulated crash");
            drop(journal);
            let cut = valid_length
                + if partial_payload {
                    HEADER_LEN as u64 + 1
                } else {
                    4
                };
            OpenOptions::new()
                .write(true)
                .open(&wal)
                .unwrap()
                .set_len(cut)
                .unwrap();
            let mut recovered = LocalWalJournal::open(&directory).expect("first recovery");
            assert_eq!(
                fs::metadata(&wal).unwrap().len(),
                valid_length,
                "open must repair physical tail, not just ignore it during replay"
            );
            assert_eq!(
                recovered
                    .append(JournalRecord::NodeEventAcknowledged {
                        node_id: 1,
                        cursor: 3,
                    })
                    .expect("append after recovery"),
                2
            );
            drop(recovered);
            let twice = LocalWalJournal::open(&directory).expect("second recovery");
            assert_eq!(twice.load_after(0).unwrap().len(), 2);
        }
    }

    #[test]
    fn failed_append_requires_reopen_before_retry() {
        let directory = temp_dir("failed-append");
        let mut journal = LocalWalJournal::open(&directory).unwrap();
        // 目录不能作为 WAL 文件写入，稳定注入 I/O 失败，不依赖 root 权限/磁盘容量。
        fs::create_dir(&journal.wal_path).unwrap();
        let record = JournalRecord::NodeEventAcknowledged {
            node_id: 1,
            cursor: 1,
        };
        assert!(journal.append(record.clone()).is_err());
        fs::remove_dir(&journal.wal_path).unwrap();
        assert!(
            journal.append(record.clone()).is_err(),
            "must not retry an uncertain tail in-place"
        );
        assert_eq!(journal.last_index(), 0);
        drop(journal);
        assert_eq!(
            LocalWalJournal::open(&directory)
                .unwrap()
                .append(record)
                .unwrap(),
            1
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "dms-local-wal-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("mkdir");
        directory
    }
}
