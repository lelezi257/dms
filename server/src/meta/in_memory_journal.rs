//! 单中心开发模式使用的进程内 MetadataJournal。

use super::metadata_journal::{
    JournalEntry, JournalError, JournalRecord, MetaSnapshot, MetadataJournal,
};

/// 只由 Meta actor 持有，因此既不需要锁，也不需要选主。
#[derive(Default)]
pub(crate) struct InMemoryJournal {
    entries: Vec<JournalEntry>,
    snapshot: Option<MetaSnapshot>,
    next_sequence: u64,
}

impl MetadataJournal for InMemoryJournal {
    fn append(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
        if self.next_sequence == 0 {
            self.next_sequence = self
                .snapshot
                .as_ref()
                .map_or(1, |snapshot| snapshot.last_applied_index + 1);
        }
        let sequence = self.next_sequence;
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
        let previous = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.last_applied_index);
        if snapshot.last_applied_index < previous {
            return Err(JournalError::InvalidSnapshot(
                "snapshot index must not move backwards",
            ));
        }
        self.next_sequence = self.next_sequence.max(snapshot.last_applied_index + 1);
        self.snapshot = Some(snapshot);
        Ok(())
    }

    fn truncate_prefix(&mut self, sequence: u64) -> Result<(), JournalError> {
        self.entries.retain(|entry| entry.sequence > sequence);
        Ok(())
    }

    fn last_index(&self) -> u64 {
        self.next_sequence
            .saturating_sub(1)
            .max(self.entries.last().map_or(0, |entry| entry.sequence))
            .max(
                self.snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.last_applied_index),
            )
    }
}

#[cfg(test)]
mod tests {
    use dms_protocol::v1 as pb;

    use super::*;

    #[test]
    fn journal_preserves_append_order_for_replay() {
        let mut journal = InMemoryJournal::default();
        let sequence = journal
            .append(JournalRecord::VersionCommitted {
                key: b"checkpoint/latest".to_vec(),
                layout: pb::VersionLayout {
                    version: 1,
                    logical_length: 0,
                    extents: Vec::new(),
                    digest: Vec::new(),
                    kind: pb::VersionKind::Tombstone as i32,
                },
                modified_time_unix_millis: 0,
                new_replicas: Vec::new(),
                operation_id: b"op-1".to_vec(),
                operation_digest: b"digest-1".to_vec(),
                commit_sequence: None,
            })
            .expect("append");

        assert_eq!(sequence, 1);
        assert!(journal.load_after(1).expect("tail").is_empty());
        assert_eq!(journal.load_after(0).expect("all").len(), 1);
    }

    #[test]
    fn snapshot_truncates_prefix_without_resetting_next_index() {
        let mut journal = InMemoryJournal::default();
        let first = journal
            .append(JournalRecord::VersionCommitted {
                key: b"k1".to_vec(),
                layout: pb::VersionLayout {
                    version: 1,
                    logical_length: 0,
                    extents: Vec::new(),
                    digest: Vec::new(),
                    kind: pb::VersionKind::Tombstone as i32,
                },
                modified_time_unix_millis: 0,
                new_replicas: Vec::new(),
                operation_id: b"op-1".to_vec(),
                operation_digest: b"digest-1".to_vec(),
                commit_sequence: None,
            })
            .expect("append first");
        journal
            .save_snapshot(MetaSnapshot {
                last_applied_index: first,
                version_floor: 0,
                next_session: 1,
                node_epochs: Vec::new(),
                node_commit_sequence_floors: Vec::new(),
                sessions: Vec::new(),
                replicas: Vec::new(),
                desired_replica_counts: Vec::new(),
                versions: Vec::new(),
                version_modified_times: Vec::new(),
                block_retirements: Vec::new(),
                retired_block_fences: Vec::new(),
                commit_sequences: Vec::new(),
                operations: Vec::new(),
                replica_operations: Vec::new(),
                event_high_watermark: 0,
                events: Vec::new(),
            })
            .expect("snapshot");
        journal.truncate_prefix(first).expect("truncate");

        assert!(journal.load_after(0).expect("after truncate").is_empty());
        assert_eq!(
            journal
                .append(JournalRecord::VersionCommitted {
                    key: b"k2".to_vec(),
                    layout: pb::VersionLayout {
                        version: 1,
                        logical_length: 0,
                        extents: Vec::new(),
                        digest: Vec::new(),
                        kind: pb::VersionKind::Tombstone as i32,
                    },
                    modified_time_unix_millis: 0,
                    new_replicas: Vec::new(),
                    operation_id: b"op-2".to_vec(),
                    operation_digest: b"digest-2".to_vec(),
                    commit_sequence: None,
                })
                .expect("append second"),
            2
        );
    }
}
