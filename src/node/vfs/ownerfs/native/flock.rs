//! Home flock uses the opened backing description, so native and FUSE callers
//! arbitrate in the same Linux lock namespace. POSIX process locks are separate.

use std::{
    collections::{HashMap, HashSet},
    fs::{File, TryLockError},
    io,
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
                    FileLockType::Unlock => description.file.unlock().map_err(map_io_error),
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
        let mut failure = None;
        for owner in &owners {
            let description = &state.descriptions[owner];
            match description.file.unlock() {
                Ok(()) => {
                    state.descriptions.remove(owner);
                }
                Err(error) => {
                    // Keep the pin for retry; dropping a clone does not release
                    // a lock while the original description is still open.
                    failure.get_or_insert_with(|| map_io_error(error));
                }
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
        failure.map_or(Ok(()), Err)
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
            if state.invalidated {
                drop(state);
                // Cleanup may retry a failed unlock, but cannot repopulate
                // session history after the entire table has been fenced.
                return self.release_matching(|owner, _| owner.ingress_session_id == scope);
            }
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
        let result = self.release_matching(|_, _| true);
        let mut state = self.state.lock().map_err(|_| LockError::Poisoned)?;
        // Permanent invalidation fences every replay. Exact terminal/session
        // history is no longer needed, and blocked threads must not recreate
        // it when waking. Failed unlock descriptions remain pinned for retry.
        state.waiters.clear();
        state.outcomes.clear();
        state.retired.clear();
        state.closed_scopes.clear();
        state.admission_closed = false;
        self.cv.notify_all();
        result
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
        TryLockError::Error(error) => map_io_error(error),
    }
}

fn map_io_error(error: io::Error) -> LockError {
    match error.kind() {
        io::ErrorKind::WouldBlock => LockError::WouldBlock,
        io::ErrorKind::Interrupted => LockError::Interrupted,
        _ => LockError::Kernel(error.raw_os_error().unwrap_or(libc::EIO)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::vfs::{
        Backend,
        ownerfs::{OpenFileHandle, tests::native_flock_fixture},
        types::{FileLockKind, FileLockRange},
    };
    use std::{io, sync::Arc, time::Instant};

    #[test]
    fn native_flock_kernel_errno_survives_conversion() {
        for errno in [
            libc::EBADF,
            libc::ENOLCK,
            libc::EINTR,
            libc::ENOSYS,
            libc::EINVAL,
            libc::EAGAIN,
            libc::EOPNOTSUPP,
            libc::ENOMEM,
        ] {
            let error = map_try_lock(TryLockError::Error(io::Error::from_raw_os_error(errno)));
            assert_eq!(error.errno(), errno);
            let public = crate::node::vfs::ownerfs::owner_lock_error(error);
            assert_eq!(crate::error::errno(&public), errno);
        }
        assert_eq!(map_try_lock(TryLockError::WouldBlock).errno(), libc::EAGAIN);
    }

    #[test]
    fn native_flock_invalidated_closed_scope_capacity_is_reclaimable() {
        let flock = NativeFlocks::default();
        for scope in 0..=MAX_CLOSED_SCOPES {
            flock.release_session(&format!("scope-{scope}")).unwrap();
        }
        assert!(!flock.is_idle().unwrap());
        flock.invalidate().unwrap();
        assert!(flock.is_idle().unwrap());
        assert_eq!(
            flock.cancel(LockWaiterId {
                ingress_session_id: "scope-0".into(),
                request_id: 1,
            }),
            Err(LockError::Interrupted)
        );
    }

    #[test]
    fn native_flock_failed_unlock_keeps_pin_and_reports_kernel_errno() {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let (_temp, fs, ctx, _root, file, native) = native_flock_fixture();
        let slot = fs
            .require_local()
            .unwrap()
            .open_file_handle(file.handle)
            .unwrap();
        // An actual valid O_PATH descriptor: flock/unlock fail with EBADF.
        // Do not close a Rust-owned fd behind its back or fake an unlock ACK.
        let path_only = File::options()
            .read(true)
            .custom_flags(libc::O_PATH)
            .open(format!("/proc/self/fd/{}", native.as_raw_fd()))
            .unwrap();
        let owner = FileLockOwner {
            ingress_session_id: "fault".into(),
            kernel_owner: 7,
        };
        let flock = NativeFlocks::default();
        flock
            .prepare(file.handle, &owner, path_only, Arc::downgrade(&slot))
            .unwrap();
        assert_eq!(flock.invalidate().unwrap_err().errno(), libc::EBADF);
        assert!(!flock.is_idle().unwrap(), "failed unlock pin was discarded");
        assert_eq!(
            flock.release_session("fault").unwrap_err().errno(),
            libc::EBADF
        );
        assert!(!flock.is_idle().unwrap());
        fs.release(&ctx, file.handle).unwrap();
    }

    #[test]
    fn native_flock_invalidate_drains_blocked_waiter_without_recreating_outcome() {
        let (_temp, fs, ctx, _root, file, native) = native_flock_fixture();
        let local = fs.require_local().unwrap();
        let slot = local.open_file_handle(file.handle).unwrap();
        let descriptor = {
            let slot = slot.lock().unwrap();
            let OpenFileHandle::Local(open) = &slot.file else {
                panic!("not local");
            };
            open.handle.file.try_clone_descriptor().unwrap()
        };
        let owner = FileLockOwner {
            ingress_session_id: "blocked".into(),
            kernel_owner: 7,
        };
        let id = LockWaiterId {
            ingress_session_id: "blocked".into(),
            request_id: 1,
        };
        let flock = Arc::new(NativeFlocks::default());
        flock
            .prepare(file.handle, &owner, descriptor, Arc::downgrade(&slot))
            .unwrap();
        native.try_lock().unwrap();
        let request = LockRequest {
            kind: FileLockKind::Flock,
            owner,
            pid: 7,
            range: FileLockRange {
                start: 0,
                end: u64::MAX,
            },
            lock_type: FileLockType::Write,
        };
        let worker_flock = flock.clone();
        let worker_id = id.clone();
        let worker = std::thread::spawn(move || worker_flock.setlk(request, Some(worker_id)));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !flock.state.lock().unwrap().waiters.contains_key(&id) {
            assert!(
                Instant::now() < deadline,
                "waiter did not enter kernel retry loop"
            );
            std::thread::yield_now();
        }
        flock.invalidate().unwrap();
        assert_eq!(worker.join().unwrap(), Err(LockError::Interrupted));
        assert!(
            flock.is_idle().unwrap(),
            "fenced waiter recreated terminal state"
        );
        native.unlock().unwrap();
        fs.release(&ctx, file.handle).unwrap();
    }
}
