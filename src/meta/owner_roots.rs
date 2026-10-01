//! OwnerFs root authority state machine.
//!
//! This module owns the durable OwnerRoots business transitions. RPC adapters
//! convert protobuf/auth context into these typed inputs and map returned domain
//! records back to wire replies.

use std::sync::Arc;

use afs_error::{Error, Result};

use super::store::{
    MetaEntity, MetaFuture, MetaKey, MetaRead, MetaStore, MetaTxn, OperationResult, RequestKey,
    RootAccessGrant, RootCommandAck, RootCommandRecord, RootRecord, RootReservationRecord,
    RootRight, StoreOperation, StoreRevision, TxnCondition, TxnMutation, TxnOutcome, WatchBatch,
    WatchChange, unavailable_meta_store,
};

pub trait OwnerRootAuthority: Send + Sync {
    fn reserve_root(&self, input: ReserveRootInput) -> MetaFuture<'_, ReserveRootOutput>;
    fn activate_root(&self, input: ActivateRootInput) -> MetaFuture<'_, RootAccessGrant>;
    fn abort_root(&self, input: AbortRootInput) -> MetaFuture<'_, ()>;
    fn lookup_root(&self, input: LookupRootInput) -> MetaFuture<'_, Option<RootRecord>>;
    fn list_owner_roots(&self, input: ListOwnerRootsInput) -> MetaFuture<'_, ListOwnerRootsOutput>;
    fn acquire_root(&self, input: AcquireRootInput) -> MetaFuture<'_, RootAccessGrant>;
    fn validate_root_access(
        &self,
        input: ValidateRootAccessInput,
    ) -> MetaFuture<'_, ValidateRootAccessOutput>;
    fn watch_root_commands(
        &self,
        input: WatchRootCommandsInput,
    ) -> MetaFuture<'_, Vec<WatchedRootCommand>>;
    fn poll_root_commands(
        &self,
        _input: WatchRootCommandsInput,
    ) -> MetaFuture<'_, RootCommandPage> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }
    fn ack_revocation(&self, input: AckRevocationInput) -> MetaFuture<'_, u64>;
    fn recover_root(&self, input: RecoverRootInput) -> MetaFuture<'_, RootAccessGrant>;
}

pub struct MissingOwnerRootAuthority;

impl OwnerRootAuthority for MissingOwnerRootAuthority {
    fn reserve_root(&self, _input: ReserveRootInput) -> MetaFuture<'_, ReserveRootOutput> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn activate_root(&self, _input: ActivateRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn abort_root(&self, _input: AbortRootInput) -> MetaFuture<'_, ()> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn lookup_root(&self, _input: LookupRootInput) -> MetaFuture<'_, Option<RootRecord>> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn list_owner_roots(
        &self,
        _input: ListOwnerRootsInput,
    ) -> MetaFuture<'_, ListOwnerRootsOutput> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn acquire_root(&self, _input: AcquireRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn validate_root_access(
        &self,
        _input: ValidateRootAccessInput,
    ) -> MetaFuture<'_, ValidateRootAccessOutput> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn watch_root_commands(
        &self,
        _input: WatchRootCommandsInput,
    ) -> MetaFuture<'_, Vec<WatchedRootCommand>> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn ack_revocation(&self, _input: AckRevocationInput) -> MetaFuture<'_, u64> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }

    fn recover_root(&self, _input: RecoverRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async { Err(unavailable_meta_store()) })
    }
}

pub struct StoreOwnerRootAuthority {
    store: Arc<dyn MetaStore>,
}

impl StoreOwnerRootAuthority {
    pub fn new(store: Arc<dyn MetaStore>) -> Self {
        Self { store }
    }
}

impl OwnerRootAuthority for StoreOwnerRootAuthority {
    fn reserve_root(&self, input: ReserveRootInput) -> MetaFuture<'_, ReserveRootOutput> {
        Box::pin(async move {
            let caller = format!("{}/{}", input.preferred_home_node_id, input.session_id);
            let request_id = input.request_id;
            let key = request_key(caller, request_id.clone());
            let existing = self
                .store
                .read(MetaRead::RootReservation {
                    root_id: input.root_id.clone(),
                })
                .await?;
            if let Some(MetaEntity::RootReservation(reservation)) = existing.entity
                && reservation.home_node_id == input.preferred_home_node_id
                && reservation.home_session_id == input.session_id
                && reservation.create_intent_id == input.create_intent_id
            {
                return Ok(ReserveRootOutput {
                    reservation,
                    already_reserved_by_same_intent: true,
                });
            }
            let reservation = RootReservationRecord {
                root_id: input.root_id.clone(),
                root_epoch: input.expected_root_epoch.max(1),
                home_node_id: input.preferred_home_node_id.clone(),
                home_session_id: input.session_id.clone(),
                create_intent_id: input.create_intent_id.clone(),
                prepare_token: format!(
                    "prepare:{}:{}:{}:{}",
                    input.root_id, input.session_id, input.create_intent_id, request_id
                ),
                created_at_revision: StoreRevision::ZERO,
            };
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::ReserveRoot);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::NodeSessionCurrent {
                    node_id: input.preferred_home_node_id,
                    session_id: input.session_id,
                },
                TxnCondition::Missing(MetaKey::Root {
                    root_id: input.root_id.clone(),
                }),
                TxnCondition::Missing(MetaKey::RootReservation {
                    root_id: input.root_id,
                }),
            ]);
            txn.mutations.extend([
                TxnMutation::Put(MetaEntity::RootReservation(reservation.clone())),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::RootReservation(reservation.clone()),
                }),
            ]);
            let OperationResult::RootReservation(reservation) =
                commit_or_replay(self.store.as_ref(), txn).await?
            else {
                return Err(invalid("reserve replay returned wrong result type"));
            };
            Ok(ReserveRootOutput {
                reservation,
                already_reserved_by_same_intent: false,
            })
        })
    }

    fn activate_root(&self, input: ActivateRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async move {
            let key = request_key(
                format!(
                    "{}/{}",
                    input.reservation.home_node_id, input.reservation.session_id
                ),
                input.request_id,
            );
            let snapshot = self
                .store
                .read(MetaRead::RootReservation {
                    root_id: input.reservation.root_id.clone(),
                })
                .await?;
            let Some(MetaEntity::RootReservation(record)) = snapshot.entity else {
                return Err(invalid("root reservation was not found"));
            };
            if record.root_epoch != input.reservation.root_epoch
                || record.home_node_id != input.reservation.home_node_id
                || record.home_session_id != input.reservation.session_id
                || record.create_intent_id != input.reservation.create_intent_id
                || record.prepare_token != input.reservation.prepare_token
            {
                return Err(invalid("reservation does not match durable reserve record"));
            }
            let root = RootRecord {
                root_id: record.root_id.clone(),
                root_epoch: record.root_epoch,
                home_node_id: record.home_node_id.clone(),
                home_session_id: record.home_session_id.clone(),
                local_prepare_id: input.local_prepare_id,
                created_at_revision: StoreRevision::ZERO,
                updated_at_revision: StoreRevision::ZERO,
            };
            let grant = home_grant(&root, 1);
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::ActivateRoot);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::NodeSessionCurrent {
                    node_id: root.home_node_id.clone(),
                    session_id: root.home_session_id.clone(),
                },
                TxnCondition::Missing(MetaKey::Root {
                    root_id: root.root_id.clone(),
                }),
                TxnCondition::EntityEquals(MetaEntity::RootReservation(record.clone())),
            ]);
            txn.mutations.extend([
                TxnMutation::Put(MetaEntity::Root(root)),
                TxnMutation::Put(MetaEntity::RootGrant(grant.clone())),
                TxnMutation::Delete(MetaKey::RootReservation {
                    root_id: grant.root_id.clone(),
                }),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::RootGrant(grant.clone()),
                }),
            ]);
            let OperationResult::RootGrant(grant) =
                commit_or_replay(self.store.as_ref(), txn).await?
            else {
                return Err(invalid("activate replay returned wrong result type"));
            };
            Ok(grant)
        })
    }

    fn abort_root(&self, input: AbortRootInput) -> MetaFuture<'_, ()> {
        Box::pin(async move {
            let snapshot = self
                .store
                .read(MetaRead::RootReservation {
                    root_id: input.root_id.clone(),
                })
                .await?;
            let Some(MetaEntity::RootReservation(reservation)) = snapshot.entity else {
                return Err(invalid("root reservation was not found"));
            };
            if reservation.home_session_id != input.session_id {
                return Err(invalid("abort session does not match durable reservation"));
            }
            if reservation.root_epoch != input.root_epoch
                || reservation.create_intent_id != input.create_intent_id
                || reservation.prepare_token != input.prepare_token
            {
                return Err(invalid("abort request does not match durable reservation"));
            }
            if let Some(authenticated) = &input.authenticated_home_node_id
                && authenticated != &reservation.home_node_id
            {
                return Err(permission_denied(format!(
                    "authenticated node {authenticated} does not match reservation home {}",
                    reservation.home_node_id
                )));
            }
            let key = request_key(
                format!("{}/{}", reservation.home_node_id, input.session_id),
                input.request_id,
            );
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::AbortRoot);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::EntityEquals(MetaEntity::RootReservation(reservation)),
            ]);
            txn.mutations.extend([
                TxnMutation::Delete(MetaKey::RootReservation {
                    root_id: input.root_id,
                }),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::Empty,
                }),
            ]);
            let _ = commit_or_replay(self.store.as_ref(), txn).await?;
            Ok(())
        })
    }

    fn lookup_root(&self, input: LookupRootInput) -> MetaFuture<'_, Option<RootRecord>> {
        Box::pin(async move {
            let snapshot = self
                .store
                .read(MetaRead::Root {
                    root_id: input.root_id,
                })
                .await?;
            Ok(match snapshot.entity {
                Some(MetaEntity::Root(root)) => Some(root),
                _ => None,
            })
        })
    }

    fn list_owner_roots(&self, input: ListOwnerRootsInput) -> MetaFuture<'_, ListOwnerRootsOutput> {
        Box::pin(async move {
            let scan = self
                .store
                .scan_owner_roots_for_recovery(&input.home_node_id)
                .await?;
            Ok(ListOwnerRootsOutput {
                authority_revision: scan.revision,
                active_roots: scan.roots,
                pending_reservations: scan.reservations,
            })
        })
    }

    fn acquire_root(&self, input: AcquireRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async move {
            let root = self
                .store
                .read(MetaRead::Root {
                    root_id: input.root_id.clone(),
                })
                .await?;
            let Some(MetaEntity::Root(root)) = root.entity else {
                return Err(invalid("root was not found"));
            };
            if input.expected_root_epoch != 0 && input.expected_root_epoch != root.root_epoch {
                return Err(invalid("root epoch changed"));
            }
            let home_grant = self
                .store
                .read(MetaRead::RootGrantByHolder {
                    root_id: root.root_id.clone(),
                    holder_node_id: root.home_node_id.clone(),
                    holder_session_id: root.home_session_id.clone(),
                })
                .await?;
            let Some(MetaEntity::RootGrant(home_grant)) = home_grant.entity else {
                return Err(invalid("home grant was not found"));
            };
            if input.expected_access_generation != 0
                && input.expected_access_generation != home_grant.access_generation
            {
                return Err(invalid("root access generation changed"));
            }
            let existing_holder_grant = self
                .store
                .read(MetaRead::RootGrantByHolder {
                    root_id: root.root_id.clone(),
                    holder_node_id: input.requester_node_id.clone(),
                    holder_session_id: input.session_id.clone(),
                })
                .await?;
            let (existing_holder_revision, existing_rights) = match existing_holder_grant.entity {
                Some(MetaEntity::RootGrant(grant)) => {
                    let rights = if grant.root_epoch == root.root_epoch
                        && grant.home_session_id == root.home_session_id
                        && grant.access_generation == home_grant.access_generation
                    {
                        grant.rights
                    } else {
                        Vec::new()
                    };
                    (Some(existing_holder_grant.revision), rights)
                }
                None => (None, Vec::new()),
                Some(_) => return Err(invalid("holder grant lookup returned wrong entity type")),
            };
            let holder_node_id = input.requester_node_id.clone();
            let holder_session_id = input.session_id.clone();
            let rights = merge_rights(existing_rights, input.rights);
            let grant = RootAccessGrant {
                root_id: root.root_id.clone(),
                root_epoch: root.root_epoch,
                home_node_id: root.home_node_id,
                home_session_id: root.home_session_id,
                holder_node_id: holder_node_id.clone(),
                holder_session_id: holder_session_id.clone(),
                access_generation: home_grant.access_generation,
                rights,
                fencing_token: format!(
                    "fence:{}:{}:{}",
                    root.root_id, holder_session_id, home_grant.access_generation
                ),
                issued_at_revision: StoreRevision::ZERO,
            };
            let key = request_key(
                format!("{}/{}", holder_node_id, holder_session_id),
                input.request_id,
            );
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::AcquireRoot);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::NodeSessionCurrent {
                    node_id: holder_node_id.clone(),
                    session_id: holder_session_id.clone(),
                },
                TxnCondition::RootEpochEquals {
                    root_id: grant.root_id.clone(),
                    root_epoch: grant.root_epoch,
                },
            ]);
            if let Some(revision) = existing_holder_revision {
                txn.conditions.push(TxnCondition::RevisionEquals {
                    key: MetaKey::RootGrant {
                        root_id: grant.root_id.clone(),
                        holder_node_id,
                        holder_session_id,
                    },
                    revision,
                });
            } else {
                txn.conditions
                    .push(TxnCondition::Missing(MetaKey::RootGrant {
                        root_id: grant.root_id.clone(),
                        holder_node_id,
                        holder_session_id,
                    }));
            }
            txn.mutations.extend([
                TxnMutation::Put(MetaEntity::RootGrant(grant.clone())),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::RootGrant(grant.clone()),
                }),
            ]);
            let OperationResult::RootGrant(grant) =
                commit_or_replay(self.store.as_ref(), txn).await?
            else {
                return Err(invalid("acquire replay returned wrong result type"));
            };
            Ok(grant)
        })
    }

    fn validate_root_access(
        &self,
        input: ValidateRootAccessInput,
    ) -> MetaFuture<'_, ValidateRootAccessOutput> {
        Box::pin(async move {
            let snapshot = self
                .store
                .read(MetaRead::RootGrant {
                    root_id: input.root_id,
                    root_epoch: input.root_epoch,
                    home_session_id: input.home_session_id,
                    holder_node_id: input.holder_node_id,
                    holder_session_id: input.holder_session_id,
                    access_generation: input.access_generation,
                    fencing_token: input.fencing_token,
                })
                .await?;
            let Some(MetaEntity::RootGrant(grant)) = snapshot.entity else {
                return Err(invalid("presented root access is not current"));
            };
            Ok(ValidateRootAccessOutput {
                grant,
                authority_revision: snapshot.revision,
            })
        })
    }

    fn poll_root_commands(&self, input: WatchRootCommandsInput) -> MetaFuture<'_, RootCommandPage> {
        Box::pin(async move {
            const LIMIT: usize = 1024;
            let view = self.store.read_view().await?;
            let authority_revision = view.revision()?.0;
            let current = view
                .read(MetaRead::CurrentNodeSession {
                    node_id: input.node_id.clone(),
                })
                .await?;
            let Some(MetaEntity::NodeSession(session)) = current.entity else {
                return Err(invalid("command poll requires a registered Node session"));
            };
            if session.session_id != input.session_id || !session.is_live_at_unix_ms(now_unix_ms())
            {
                return Err(invalid(
                    "command poll requires the current live Node session",
                ));
            }
            if input.after_revision > authority_revision {
                return Err(invalid("command poll cursor is ahead of authority"));
            }
            let batch = view
                .watch(StoreRevision(input.after_revision), LIMIT)
                .await?;
            batch.validate_order()?;
            let WatchBatch::Events {
                start_revision,
                next_revision,
                events,
            } = batch
            else {
                return Err(invalid(
                    "command history was compacted; native admission requires reconciliation",
                ));
            };
            if start_revision.0 != input.after_revision
                || next_revision.0 <= start_revision.0
                || next_revision.0 > authority_revision.saturating_add(1)
            {
                return Err(invalid("command poll returned an invalid processed prefix"));
            }
            let commands = events
                .into_iter()
                .filter_map(|event| match event.change {
                    WatchChange::Put(MetaEntity::RootCommand(command))
                        if command.home_node_id == input.node_id
                            && command.home_session_id == input.session_id =>
                    {
                        Some(WatchedRootCommand {
                            command,
                            revision: event.revision,
                        })
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            // Store event limits preserve complete transactions and are soft.
            // Never truncate a command transaction or advance past unreturned work.
            if commands.len() > LIMIT {
                return Err(invalid(
                    "atomic command batch exceeds the bounded consumer capacity",
                ));
            }
            Ok(RootCommandPage {
                node_id: input.node_id,
                session_id: input.session_id,
                resume_after_revision: next_revision.0 - 1,
                authority_revision,
                commands,
            })
        })
    }

    fn watch_root_commands(
        &self,
        input: WatchRootCommandsInput,
    ) -> MetaFuture<'_, Vec<WatchedRootCommand>> {
        Box::pin(async move {
            let batch = self
                .store
                .watch(StoreRevision(input.after_revision), 1024)
                .await?;
            match batch {
                WatchBatch::Events { events, .. } => Ok(events
                    .into_iter()
                    .filter_map(|event| match event.change {
                        WatchChange::Put(MetaEntity::RootCommand(command))
                            if command.home_node_id == input.node_id
                                && command.home_session_id == input.session_id =>
                        {
                            Some(WatchedRootCommand {
                                command,
                                revision: event.revision,
                            })
                        }
                        _ => None,
                    })
                    .collect()),
                WatchBatch::Compacted { .. } => {
                    Err(invalid("watch history was compacted; caller must recover"))
                }
            }
        })
    }

    fn ack_revocation(&self, input: AckRevocationInput) -> MetaFuture<'_, u64> {
        Box::pin(async move {
            let ack = RootCommandAck {
                command_id: input.command_id,
                node_id: input.node_id.clone(),
                session_id: input.session_id.clone(),
                root_id: input.root_id,
                root_epoch: input.root_epoch,
                access_generation: input.access_generation,
                success: input.success,
                message: input.message,
            };
            let key = request_key(
                format!("{}/{}", input.node_id, input.session_id),
                input.request_id,
            );
            let previous = self
                .store
                .read(MetaRead::RequestOutcome(key.clone()))
                .await?;
            if let Some(previous) = previous.request_outcome {
                if previous.operation != StoreOperation::AckRootCommand
                    || previous.result != OperationResult::RootCommandAck(ack.clone())
                {
                    return Err(invalid(
                        "ACK request replay does not match its recorded payload",
                    ));
                }
                return Ok(now_unix_ms());
            }
            let command = self
                .store
                .read(MetaRead::RootCommand {
                    command_id: ack.command_id.clone(),
                })
                .await?;
            let Some(MetaEntity::RootCommand(command)) = command.entity else {
                return Err(invalid("revocation command was not found"));
            };
            if command.home_node_id != ack.node_id
                || command.home_session_id != ack.session_id
                || command.root_id != ack.root_id
                || command.root_epoch != ack.root_epoch
                || command.old_access_generation != ack.access_generation
            {
                return Err(invalid("ACK does not match the pending revocation command"));
            }
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::AckRootCommand);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::EntityEquals(MetaEntity::RootCommand(command)),
                TxnCondition::NodeSessionCurrent {
                    node_id: ack.node_id.clone(),
                    session_id: ack.session_id.clone(),
                },
                TxnCondition::Missing(MetaKey::RootCommandAck {
                    command_id: ack.command_id.clone(),
                    home_node_id: ack.node_id.clone(),
                    home_session_id: ack.session_id.clone(),
                }),
            ]);
            txn.mutations.extend([
                TxnMutation::Put(MetaEntity::RootCommandAck(ack.clone())),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::RootCommandAck(ack.clone()),
                }),
            ]);
            let recorded = commit_or_replay(self.store.as_ref(), txn).await?;
            if recorded != OperationResult::RootCommandAck(ack) {
                return Err(invalid(
                    "ACK request replay does not match its recorded payload",
                ));
            }
            Ok(now_unix_ms())
        })
    }

    fn recover_root(&self, input: RecoverRootInput) -> MetaFuture<'_, RootAccessGrant> {
        Box::pin(async move {
            let root = self
                .store
                .read(MetaRead::Root {
                    root_id: input.root_id.clone(),
                })
                .await?;
            let Some(MetaEntity::Root(mut root)) = root.entity else {
                return Err(invalid("root was not found"));
            };
            if root.home_node_id != input.home_node_id
                || root.root_epoch != input.expected_root_epoch
                || root.local_prepare_id != input.local_prepare_id
            {
                return Err(invalid("recover request does not match durable root"));
            }
            let previous = self
                .store
                .read(MetaRead::RootGrantByHolder {
                    root_id: root.root_id.clone(),
                    holder_node_id: root.home_node_id.clone(),
                    holder_session_id: root.home_session_id.clone(),
                })
                .await?;
            let next_generation = match previous.entity {
                Some(MetaEntity::RootGrant(grant)) => grant.access_generation.saturating_add(1),
                _ => 1,
            };
            root.home_session_id = input.home_session_id.clone();
            let grant = home_grant(&root, next_generation);
            let key = request_key(
                format!("{}/{}", root.home_node_id, root.home_session_id),
                input.request_id,
            );
            let mut txn = MetaTxn::new(key.clone(), StoreOperation::RecoverRoot);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(key),
                TxnCondition::NodeSessionCurrent {
                    node_id: root.home_node_id.clone(),
                    session_id: root.home_session_id.clone(),
                },
                TxnCondition::RootEpochEquals {
                    root_id: root.root_id.clone(),
                    root_epoch: root.root_epoch,
                },
            ]);
            txn.mutations.extend([
                TxnMutation::Put(MetaEntity::Root(root)),
                TxnMutation::Put(MetaEntity::RootGrant(grant.clone())),
                TxnMutation::RecordRequestOutcome(super::store::RequestOutcome {
                    request: txn.request.clone(),
                    operation: txn.operation,
                    result: OperationResult::RootGrant(grant.clone()),
                }),
            ]);
            let OperationResult::RootGrant(grant) =
                commit_or_replay(self.store.as_ref(), txn).await?
            else {
                return Err(invalid("recover replay returned wrong result type"));
            };
            Ok(grant)
        })
    }
}

pub struct ReserveRootInput {
    pub request_id: String,
    pub root_id: String,
    pub preferred_home_node_id: String,
    pub session_id: String,
    pub rights: Vec<RootRight>,
    pub expected_root_epoch: u64,
    pub create_intent_id: String,
}

pub struct ReserveRootOutput {
    pub reservation: RootReservationRecord,
    pub already_reserved_by_same_intent: bool,
}

pub struct ActivateRootInput {
    pub request_id: String,
    pub reservation: WireRootReservation,
    pub local_prepare_id: String,
}

pub struct WireRootReservation {
    pub root_id: String,
    pub root_epoch: u64,
    pub home_node_id: String,
    pub session_id: String,
    pub create_intent_id: String,
    pub prepare_token: String,
}

pub struct AbortRootInput {
    pub request_id: String,
    pub root_id: String,
    pub root_epoch: u64,
    pub session_id: String,
    pub create_intent_id: String,
    pub prepare_token: String,
    pub authenticated_home_node_id: Option<String>,
}

pub struct LookupRootInput {
    pub root_id: String,
}

pub struct ListOwnerRootsInput {
    pub home_node_id: String,
}

pub struct ListOwnerRootsOutput {
    pub authority_revision: StoreRevision,
    pub active_roots: Vec<RootRecord>,
    pub pending_reservations: Vec<RootReservationRecord>,
}

pub struct AcquireRootInput {
    pub request_id: String,
    pub root_id: String,
    pub requester_node_id: String,
    pub session_id: String,
    pub rights: Vec<RootRight>,
    pub expected_root_epoch: u64,
    pub expected_access_generation: u64,
}

pub struct ValidateRootAccessInput {
    pub root_id: String,
    pub root_epoch: u64,
    pub home_session_id: String,
    pub holder_node_id: String,
    pub holder_session_id: String,
    pub access_generation: u64,
    pub fencing_token: String,
}

pub struct ValidateRootAccessOutput {
    pub grant: RootAccessGrant,
    pub authority_revision: StoreRevision,
}

pub struct WatchRootCommandsInput {
    pub node_id: String,
    pub session_id: String,
    pub after_revision: u64,
}

pub struct WatchedRootCommand {
    pub command: RootCommandRecord,
    pub revision: StoreRevision,
}

/// One current-session authority observation, not an ongoing native-access lease.
/// A consumer must apply and persist the commands before advancing the cursor.
pub struct RootCommandPage {
    pub node_id: String,
    pub session_id: String,
    pub resume_after_revision: u64,
    pub authority_revision: u64,
    pub commands: Vec<WatchedRootCommand>,
}

pub struct AckRevocationInput {
    pub request_id: String,
    pub command_id: String,
    pub node_id: String,
    pub session_id: String,
    pub root_id: String,
    pub root_epoch: u64,
    pub access_generation: u64,
    pub success: bool,
    pub message: String,
}

pub struct RecoverRootInput {
    pub request_id: String,
    pub root_id: String,
    pub expected_root_epoch: u64,
    pub home_node_id: String,
    pub home_session_id: String,
    pub local_prepare_id: String,
}

async fn commit_or_replay(store: &dyn MetaStore, txn: MetaTxn) -> Result<OperationResult> {
    let operation = txn.operation;
    match store.compare_and_commit(txn).await? {
        TxnOutcome::Committed { outcome, .. } => Ok(outcome.result),
        TxnOutcome::ConditionFailed {
            existing_outcome: Some(outcome),
            ..
        } if outcome.operation == operation => Ok(outcome.result),
        TxnOutcome::ConditionFailed {
            existing_outcome: Some(_),
            ..
        } => Err(invalid(
            "request_id was already used for a different Meta operation",
        )),
        TxnOutcome::ConditionFailed { .. } => Err(invalid(
            "Meta transaction conditions failed; caller must refresh authority state",
        )),
    }
}

fn request_key(caller_id: impl Into<String>, request_id: impl Into<String>) -> RequestKey {
    RequestKey::new(caller_id, request_id)
}

fn home_grant(root: &RootRecord, access_generation: u64) -> RootAccessGrant {
    RootAccessGrant {
        root_id: root.root_id.clone(),
        root_epoch: root.root_epoch,
        home_node_id: root.home_node_id.clone(),
        home_session_id: root.home_session_id.clone(),
        holder_node_id: root.home_node_id.clone(),
        holder_session_id: root.home_session_id.clone(),
        access_generation,
        rights: vec![
            RootRight::Lookup,
            RootRight::Read,
            RootRight::Write,
            RootRight::Admin,
        ],
        fencing_token: format!(
            "fence:{}:{}:{}",
            root.root_id, root.home_session_id, access_generation
        ),
        issued_at_revision: StoreRevision::ZERO,
    }
}

fn merge_rights(
    existing: impl IntoIterator<Item = RootRight>,
    requested: impl IntoIterator<Item = RootRight>,
) -> Vec<RootRight> {
    let mut has_lookup = false;
    let mut has_read = false;
    let mut has_write = false;
    let mut has_admin = false;
    for right in existing.into_iter().chain(requested) {
        match right {
            RootRight::Lookup => has_lookup = true,
            RootRight::Read => has_read = true,
            RootRight::Write => has_write = true,
            RootRight::Admin => has_admin = true,
        }
    }
    let mut out = Vec::new();
    if has_lookup {
        out.push(RootRight::Lookup);
    }
    if has_read {
        out.push(RootRight::Read);
    }
    if has_write {
        out.push(RootRight::Write);
    }
    if has_admin {
        out.push(RootRight::Admin);
    }
    out
}

fn invalid(message: impl Into<String>) -> Error {
    Error::coded(afs_error::META_CATALOG_INVALID_REQUEST, message)
}

fn permission_denied(message: impl Into<String>) -> Error {
    Error::coded(afs_error::CLIENT_PERMISSION_DENIED, message)
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
pub(crate) mod native_control_tests {
    use super::*;
    use crate::meta::store::{NodeSessionLease, RequestOutcome, Store, memory::MemoryBackend};
    use std::time::Duration;

    pub(crate) async fn fixture() -> (Arc<Store>, StoreOwnerRootAuthority, RootCommandRecord) {
        let store = Arc::new(
            Store::open(Arc::new(MemoryBackend::default()))
                .await
                .unwrap(),
        );
        store
            .register_node_session(
                RequestKey::new("node-a", "register"),
                NodeSessionLease {
                    node_id: "node-a".into(),
                    session_id: "session-a".into(),
                    grpc_addr: "http://node-a:7400".into(),
                    data_addr: "http://node-a:7500".into(),
                    rest_addr: "http://node-a:7600".into(),
                    storage_devices: Vec::new(),
                    lease_ttl: Duration::from_secs(1800),
                },
            )
            .await
            .unwrap();
        let command = RootCommandRecord {
            command_id: "revoke-1".into(),
            home_node_id: "node-a".into(),
            home_session_id: "session-a".into(),
            root_id: "workspace-a".into(),
            root_epoch: 7,
            old_access_generation: 11,
        };
        put(
            &store,
            "seed-command",
            StoreOperation::BeginRootRevocation,
            MetaEntity::RootCommand(command.clone()),
            OperationResult::RootCommand(command.clone()),
        )
        .await;
        let service = StoreOwnerRootAuthority::new(store.clone());
        (store, service, command)
    }

    async fn put(
        store: &Store,
        request: &str,
        operation: StoreOperation,
        entity: MetaEntity,
        result: OperationResult,
    ) {
        let request = RequestKey::new("fixture", request);
        let mut txn = MetaTxn::new(request.clone(), operation);
        txn.conditions
            .push(TxnCondition::RequestAbsent(request.clone()));
        txn.mutations.extend([
            TxnMutation::Put(entity),
            TxnMutation::RecordRequestOutcome(RequestOutcome {
                request,
                operation,
                result,
            }),
        ]);
        assert!(matches!(
            store.compare_and_commit(txn).await.unwrap(),
            TxnOutcome::Committed { .. }
        ));
    }

    fn ack(command: &RootCommandRecord, request: &str) -> AckRevocationInput {
        AckRevocationInput {
            request_id: request.into(),
            command_id: command.command_id.clone(),
            node_id: command.home_node_id.clone(),
            session_id: command.home_session_id.clone(),
            root_id: command.root_id.clone(),
            root_epoch: command.root_epoch,
            access_generation: command.old_access_generation,
            success: true,
            message: "drained".into(),
        }
    }

    fn poll(after_revision: u64) -> WatchRootCommandsInput {
        WatchRootCommandsInput {
            node_id: "node-a".into(),
            session_id: "session-a".into(),
            after_revision,
        }
    }

    async fn put_commands(store: &Store, request: &str, commands: Vec<RootCommandRecord>) {
        let request = RequestKey::new("fixture", request);
        let operation = StoreOperation::BeginRootRevocation;
        let mut txn = MetaTxn::new(request.clone(), operation);
        txn.conditions
            .push(TxnCondition::RequestAbsent(request.clone()));
        let result = OperationResult::RootCommand(commands.last().unwrap().clone());
        txn.mutations.extend(
            commands
                .into_iter()
                .map(|c| TxnMutation::Put(MetaEntity::RootCommand(c))),
        );
        txn.mutations
            .push(TxnMutation::RecordRequestOutcome(RequestOutcome {
                request,
                operation,
                result,
            }));
        assert!(matches!(
            store.compare_and_commit(txn).await.unwrap(),
            TxnOutcome::Committed { .. }
        ));
    }

    #[tokio::test]
    async fn native_control_poll_advances_filtered_prefix_without_skipping_later_command() {
        let (store, service, command) = fixture().await;
        let start = store.read_view().await.unwrap().revision().unwrap().0;
        let unrelated = (0..1024)
            .map(|index| RootCommandRecord {
                command_id: format!("other-{index}"),
                home_node_id: "node-b".into(),
                ..command.clone()
            })
            .collect();
        put_commands(&store, "unrelated", unrelated).await;
        let later = RootCommandRecord {
            command_id: "later-command".into(),
            ..command
        };
        put_commands(&store, "later", vec![later.clone()]).await;
        let first = service.poll_root_commands(poll(start)).await.unwrap();
        assert!(first.commands.is_empty());
        assert!(first.resume_after_revision > start);
        assert!(first.resume_after_revision < first.authority_revision);
        let second = service
            .poll_root_commands(poll(first.resume_after_revision))
            .await
            .unwrap();
        assert_eq!(second.commands.len(), 1);
        assert_eq!(second.commands[0].command, later);
        assert_eq!(second.resume_after_revision, second.authority_revision);
        let empty = service
            .poll_root_commands(poll(second.resume_after_revision))
            .await
            .unwrap();
        assert!(empty.commands.is_empty());
        assert_eq!(empty.resume_after_revision, second.resume_after_revision);
    }

    #[tokio::test]
    async fn native_control_poll_refuses_future_cursor_and_replaced_node_session() {
        let (store, service, _command) = fixture().await;
        service.poll_root_commands(poll(0)).await.unwrap();
        let current = store.read_view().await.unwrap().revision().unwrap();
        assert!(
            service
                .poll_root_commands(poll(current.0 + 1))
                .await
                .is_err()
        );
        store
            .register_node_session(
                RequestKey::new("node-a", "replace-session"),
                NodeSessionLease {
                    node_id: "node-a".into(),
                    session_id: "new-session".into(),
                    grpc_addr: "http://node-a:7400".into(),
                    data_addr: "http://node-a:7500".into(),
                    rest_addr: "http://node-a:7600".into(),
                    storage_devices: Vec::new(),
                    lease_ttl: Duration::from_secs(1800),
                },
            )
            .await
            .unwrap();
        assert!(service.poll_root_commands(poll(0)).await.is_err());
    }

    #[tokio::test]
    async fn native_control_poll_refuses_oversized_atomic_command_batch() {
        let (store, service, command) = fixture().await;
        service.poll_root_commands(poll(0)).await.unwrap();
        let start = store.read_view().await.unwrap().revision().unwrap().0;
        put_commands(
            &store,
            "oversized",
            (0..1025)
                .map(|index| RootCommandRecord {
                    command_id: format!("revoke-{index}"),
                    ..command.clone()
                })
                .collect(),
        )
        .await;
        assert!(
            service.poll_root_commands(poll(start)).await.is_err(),
            "truncated or unbounded command batch admitted"
        );
    }

    #[tokio::test]
    async fn native_control_pinned_revision_and_watch_do_not_follow_later_commits() {
        let (store, _service, command) = fixture().await;
        let view = store.read_view().await.unwrap();
        let pinned = view.revision().unwrap();
        let session_mod = view
            .read(MetaRead::CurrentNodeSession {
                node_id: "node-a".into(),
            })
            .await
            .unwrap()
            .revision;
        assert!(
            session_mod < pinned,
            "entity mod revision was confused with snapshot watermark"
        );
        put_commands(
            &store,
            "later-view",
            vec![RootCommandRecord {
                command_id: "later-view".into(),
                ..command
            }],
        )
        .await;
        assert_eq!(view.revision().unwrap(), pinned);
        assert!(store.read_view().await.unwrap().revision().unwrap() > pinned);
        let WatchBatch::Events {
            events,
            next_revision,
            ..
        } = view.watch(pinned, 1024).await.unwrap()
        else {
            panic!("pinned snapshot must remain readable");
        };
        assert!(events.is_empty());
        assert_eq!(next_revision, pinned.next());
    }

    #[tokio::test]
    async fn native_control_poll_advances_committed_revisions_without_entity_events() {
        let (store, service, command) = fixture().await;
        let after = store.read_view().await.unwrap().revision().unwrap().0;
        let request = RequestKey::new("management", "receipt-only");
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::BeginRootRevocation);
        txn.conditions
            .push(TxnCondition::RequestAbsent(request.clone()));
        txn.mutations
            .push(TxnMutation::RecordRequestOutcome(RequestOutcome {
                request,
                operation: StoreOperation::BeginRootRevocation,
                result: OperationResult::RootCommand(command),
            }));
        assert!(matches!(
            store.compare_and_commit(txn).await.unwrap(),
            TxnOutcome::Committed { .. }
        ));
        let page = service.poll_root_commands(poll(after)).await.unwrap();
        assert!(page.commands.is_empty());
        assert!(page.authority_revision > after);
        assert_eq!(
            page.resume_after_revision, page.authority_revision,
            "empty logical revisions stalled the processed-prefix cursor"
        );
    }

    #[tokio::test]
    async fn native_revocation_ack_refuses_other_command_root_epoch_or_generation() {
        for field in 0..4 {
            let (_store, service, command) = fixture().await;
            let mut input = ack(&command, "wrong");
            match field {
                0 => input.command_id = "absent-command".into(),
                1 => input.root_id = "workspace-b".into(),
                2 => input.root_epoch += 1,
                3 => input.access_generation += 1,
                _ => unreachable!(),
            }
            assert!(
                service.ack_revocation(input).await.is_err(),
                "accepted mismatched ACK field {field}"
            );
        }
    }

    #[tokio::test]
    async fn native_revocation_ack_replays_exact_payload_but_refuses_changed_payload() {
        let (_store, service, command) = fixture().await;
        service.ack_revocation(ack(&command, "ack")).await.unwrap();
        service.ack_revocation(ack(&command, "ack")).await.unwrap();
        let mut changed = ack(&command, "ack");
        changed.success = false;
        assert!(
            service.ack_revocation(changed).await.is_err(),
            "request replay accepted changed ACK outcome"
        );
    }

    #[tokio::test]
    async fn native_revocation_ack_exact_replay_survives_command_retirement() {
        let (store, service, command) = fixture().await;
        service.ack_revocation(ack(&command, "ack")).await.unwrap();
        let request = RequestKey::new("management", "retire-command");
        let mut txn = MetaTxn::new(request.clone(), StoreOperation::CommitRootRevocation);
        txn.conditions.extend([
            TxnCondition::RequestAbsent(request.clone()),
            TxnCondition::RootCommandAcked {
                command_id: command.command_id.clone(),
                node_id: command.home_node_id.clone(),
                session_id: command.home_session_id.clone(),
            },
        ]);
        txn.mutations.extend([
            TxnMutation::Delete(MetaKey::RootCommand {
                command_id: command.command_id.clone(),
            }),
            TxnMutation::RecordRequestOutcome(RequestOutcome {
                request,
                operation: StoreOperation::CommitRootRevocation,
                result: OperationResult::RootCommand(command.clone()),
            }),
        ]);
        assert!(matches!(
            store.compare_and_commit(txn).await.unwrap(),
            TxnOutcome::Committed { .. }
        ));
        service
            .ack_revocation(ack(&command, "ack"))
            .await
            .expect("exact historical ACK receipt must remain replayable");
        assert!(
            service
                .ack_revocation(ack(&command, "new-ack"))
                .await
                .is_err(),
            "new ACK created for retired command"
        );
    }

    #[tokio::test]
    async fn native_revocation_commit_requires_ack_for_exact_pending_command() {
        for field in 0..4 {
            let (store, _service, command) = fixture().await;
            let mut bad = RootCommandAck {
                command_id: command.command_id.clone(),
                node_id: command.home_node_id.clone(),
                session_id: command.home_session_id.clone(),
                root_id: command.root_id.clone(),
                root_epoch: command.root_epoch,
                access_generation: command.old_access_generation,
                success: true,
                message: "drained".into(),
            };
            match field {
                0 => bad.root_id = "workspace-b".into(),
                1 => bad.root_epoch += 1,
                2 => bad.access_generation += 1,
                3 => bad.success = false,
                _ => unreachable!(),
            }
            put(
                &store,
                "inject-ack",
                StoreOperation::AckRootCommand,
                MetaEntity::RootCommandAck(bad.clone()),
                OperationResult::RootCommandAck(bad),
            )
            .await;
            let request = RequestKey::new("management", "commit");
            let mut txn = MetaTxn::new(request.clone(), StoreOperation::CommitRootRevocation);
            txn.conditions.extend([
                TxnCondition::RequestAbsent(request.clone()),
                TxnCondition::RootCommandAcked {
                    command_id: command.command_id.clone(),
                    node_id: command.home_node_id.clone(),
                    session_id: command.home_session_id.clone(),
                },
            ]);
            txn.mutations
                .push(TxnMutation::RecordRequestOutcome(RequestOutcome {
                    request,
                    operation: StoreOperation::CommitRootRevocation,
                    result: OperationResult::RootCommand(command),
                }));
            assert!(
                matches!(
                    store.compare_and_commit(txn).await.unwrap(),
                    TxnOutcome::ConditionFailed { .. }
                ),
                "commit accepted mismatched ACK field {field}"
            );
        }
    }
}
