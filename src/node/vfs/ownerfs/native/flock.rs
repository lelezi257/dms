//! Home flock uses the opened backing description, so native and FUSE callers
//! arbitrate in the same Linux lock namespace. POSIX process locks are separate.

use std::{
    collections::{HashMap, HashSet},
    fs::{File, TryLockError},
    sync::{Condvar, Mutex, Weak},
    time::Duration,
};

use super::super::{super::types::FileHandle, OpenFileHandleSlot};
use crate::node::vfs::{
    locks::{LockError, LockRequest, LockWaiterId, LockWaiterOutcome},
    types::{FileLockOwner, FileLockType},
};

const MAX_DESCRIPTIONS: usize = 1024;
const MAX_WAITERS: usize = 64;
const MAX_OUTCOMES: usize = 128;
const MAX_CLOSED_SCOPES: usize = 128;

struct Description {
    handle: FileHandle,
    file: File,
    slot: Weak<Mutex<OpenFileHandleSlot>>,
}

#[derive(Default)]
struct State {
    descriptions: HashMap<FileLockOwner, Description>,
    waiters: HashMap<LockWaiterId, FileLockOwner>,
    outcomes: HashMap<LockWaiterId, (LockWaiterOutcome, Option<FileLockOwner>)>,
    retired: HashSet<LockWaiterId>,
    closed_scopes: HashSet<String>,
    admission_closed: bool,
    invalidated: bool,
}

#[derive(Default)]
pub(in crate::node::vfs::ownerfs) struct NativeFlocks {
    state: Mutex<State>,
    cv: Condvar,
}

impl NativeFlocks {
    pub(super) fn prepare(
        &self,
        handle: FileHandle,
        owner: &FileLockOwner,
        file: File,
        slot: Weak<Mutex<OpenFileHandleSlot>>,
    ) -> Result<(), LockError> {
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        check_scope(&state, &owner.ingress_session_id)?;
        if let Some(existing) = state.descriptions.get(owner) {
            return if existing.handle == handle {
                Ok(())
            } else {
                Err(LockError::InvalidRange)
            };
        }
        // A flock owner represents one opened description. Arbitrary token
        // changes cannot make cleanup of one owner unlock another live owner.
        if state.descriptions.values().any(|old| old.handle == handle) {
            return Err(LockError::InvalidRange);
        }
        if state.descriptions.len() >= MAX_DESCRIPTIONS {
            return Err(LockError::Capacity);
        }
        state
            .descriptions
            .insert(owner.clone(), Description { handle, file, slot });
        Ok(())
    }

    pub(super) fn setlk(
        &self,
        request: LockRequest,
        waiter: Option<LockWaiterId>,
    ) -> Result<(), LockError> {
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        check_scope(&state, &request.owner.ingress_session_id)?;
        if let Some(id) = &waiter {
            if id.ingress_session_id != request.owner.ingress_session_id {
                return Err(LockError::InvalidRange);
            }
            match state.outcomes.get(id).map(|(outcome, _)| outcome) {
                Some(LockWaiterOutcome::Cancelled) => return Err(LockError::Interrupted),
                Some(_) => return Err(LockError::DuplicateWaiter),
                None => {}
            }
            if state.retired.contains(id) || state.waiters.contains_key(id) {
                return Err(LockError::DuplicateWaiter);
            }
            if state.waiters.len() >= MAX_WAITERS || terminal_budget(&state) >= MAX_OUTCOMES {
                return Err(LockError::Capacity);
            }
            state.waiters.insert(id.clone(), request.owner.clone());
        }
        loop {
            let result = check_scope(&state, &request.owner.ingress_session_id).and_then(|()| {
                if let Some(id) = &waiter
                    && !state.waiters.contains_key(id)
                {
                    return Err(LockError::Interrupted);
                }
                let description = state
                    .descriptions
                    .get(&request.owner)
                    .ok_or(LockError::Interrupted)?;
                // Lock ordering: coordinator -> slot. Close marks the slot
                // closed, drops its mutex, then enters this coordinator.
                // Holding the slot through the NB syscall closes the race
                // where a final release could be followed by a late grant.
                let slot = description.slot.upgrade().ok_or(LockError::Interrupted)?;
                let slot = slot.lock().map_err(|_| LockError::Poisoned)?;
                if slot.closed {
                    return Err(LockError::Interrupted);
                }
                match request.lock_type {
                    FileLockType::Read => description.file.try_lock_shared().map_err(map_try_lock),
                    FileLockType::Write => description.file.try_lock().map_err(map_try_lock),
                    FileLockType::Unlock => {
                        description.file.unlock().map_err(|_| LockError::Poisoned)
                    }
                }
            });
            match result {
                Err(LockError::WouldBlock) if waiter.is_some() => {
                    // Native processes do not signal our condvar. This timeout
                    // schedules another actual kernel attempt; elapsed time
                    // never serves as evidence that the lock was granted.
                    state = self
                        .cv
                        .wait_timeout(state, Duration::from_millis(10))
                        .map_err(|_| LockError::Poisoned)?
                        .0;
                }
                result => {
                    if let Some(id) = &waiter
                        && state.waiters.remove(id).is_some()
                    {
                        if result.is_ok() {
                            state.outcomes.insert(
                                id.clone(),
                                (LockWaiterOutcome::Granted, Some(request.owner.clone())),
                            );
                        } else if result == Err(LockError::Interrupted) {
                            state.outcomes.insert(
                                id.clone(),
                                (LockWaiterOutcome::Cancelled, Some(request.owner.clone())),
                            );
                        }
                    }
                    self.cv.notify_all();
                    return result;
                }
            }
        }
    }

    pub(super) fn cancel(&self, id: LockWaiterId) -> Result<LockWaiterOutcome, LockError> {
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        if let Some((outcome, _)) = state.outcomes.get(&id) {
            return Ok(*outcome);
        }
        if state.retired.contains(&id) {
            return Ok(LockWaiterOutcome::Unknown);
        }
        check_scope(&state, &id.ingress_session_id)?;
        let owner = state.waiters.remove(&id);
        if owner.is_none() && terminal_budget(&state) >= MAX_OUTCOMES {
            return Err(LockError::Capacity);
        }
        let outcome = if owner.is_some() {
            LockWaiterOutcome::Cancelled
        } else {
            LockWaiterOutcome::Unknown
        };
        state
            .outcomes
            .insert(id, (LockWaiterOutcome::Cancelled, owner));
        self.cv.notify_all();
        Ok(outcome)
    }

    pub(super) fn acknowledge(
        &self,
        id: &LockWaiterId,
    ) -> Result<Option<LockWaiterOutcome>, LockError> {
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        if let Some((outcome, _)) = state.outcomes.remove(id) {
            state.retired.insert(id.clone());
            Ok(Some(outcome))
        } else {
            Ok(None)
        }
    }

    fn release_matching(
        &self,
        matches: impl Fn(&FileLockOwner, &Description) -> bool,
    ) -> Result<(), LockError> {
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        let owners = state
            .descriptions
            .iter()
            .filter(|(owner, description)| matches(owner, description))
            .map(|(owner, _)| owner.clone())
            .collect::<HashSet<_>>();
        let mut failed = false;
        for owner in &owners {
            let description = &state.descriptions[owner];
            if description.file.unlock().is_ok() {
                state.descriptions.remove(owner);
            } else {
                // Keep the pin for retry; dropping a clone does not release a
                // lock while the original Home descriptor is still open.
                failed = true;
            }
        }
        let waiting = state
            .waiters
            .iter()
            .filter(|(_, owner)| owners.contains(*owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in waiting {
            let owner = state.waiters.remove(&id);
            state
                .outcomes
                .insert(id, (LockWaiterOutcome::Cancelled, owner));
        }
        for (outcome, owner) in state.outcomes.values_mut() {
            if *outcome == LockWaiterOutcome::Granted
                && owner.as_ref().is_some_and(|owner| owners.contains(owner))
            {
                *outcome = LockWaiterOutcome::Cancelled;
            }
        }
        self.cv.notify_all();
        if failed {
            Err(LockError::Poisoned)
        } else {
            Ok(())
        }
    }

    pub(super) fn release_owner(&self, owner: &FileLockOwner) -> Result<(), LockError> {
        self.release_matching(|candidate, _| candidate == owner)
    }

    pub(super) fn release_handle(&self, handle: FileHandle) -> Result<(), LockError> {
        self.release_matching(|_, description| description.handle == handle)
    }

    pub(super) fn release_session(&self, scope: &str) -> Result<(), LockError> {
        {
            let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
            if !state.closed_scopes.contains(scope)
                && state.closed_scopes.len() >= MAX_CLOSED_SCOPES
            {
                state.admission_closed = true;
            } else {
                state.closed_scopes.insert(scope.into());
            }
        }
        let result = self.release_matching(|owner, _| owner.ingress_session_id == scope);
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        state
            .outcomes
            .retain(|id, _| id.ingress_session_id != scope);
        state.retired.retain(|id| id.ingress_session_id != scope);
        self.cv.notify_all();
        result
    }

    pub(super) fn invalidate(&self) -> Result<(), LockError> {
        self.state
            .lock()
            .map_err(|_| LockError::Poisoned)?
            .invalidated = true;
        self.release_matching(|_, _| true)
    }

    pub(super) fn is_idle(&self) -> Result<bool, LockError> {
        let state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        Ok(state.descriptions.is_empty()
            && state.waiters.is_empty()
            && state.outcomes.is_empty()
            && state.retired.is_empty()
            && !state.admission_closed)
    }
}

fn check_scope(state: &State, scope: &str) -> Result<(), LockError> {
    if state.invalidated || state.closed_scopes.contains(scope) {
        Err(LockError::Interrupted)
    } else if state.admission_closed
        && !state
            .descriptions
            .keys()
            .any(|owner| owner.ingress_session_id == scope)
    {
        Err(LockError::Capacity)
    } else {
        Ok(())
    }
}

fn terminal_budget(state: &State) -> usize {
    state.outcomes.len() + state.retired.len() + state.waiters.len()
}

fn map_try_lock(error: TryLockError) -> LockError {
    match error {
        TryLockError::WouldBlock => LockError::WouldBlock,
        TryLockError::Error(_) => LockError::Poisoned,
    }
}
