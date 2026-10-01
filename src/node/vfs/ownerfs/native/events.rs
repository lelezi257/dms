//! Post-reply hints for the independent workspace lifecycle worker.
//! A hint contains no authority, epoch, source path or mount claim. Consumers
//! must revalidate current Home authority and prepared physical identities.
use crate::node::vfs::types::BackendInode;
use std::{
    ffi::OsString,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    time::Duration,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCreated {
    /// Raw component from a successful top-level mkdir, never a source path.
    pub name: OsString,
    /// Opaque identity in this OwnerFs/FUSE session, not a durable disk inode.
    pub inode: BackendInode,
}

#[derive(Clone)]
pub struct WorkspaceEventSender {
    sender: SyncSender<WorkspaceCreated>,
    rescan: Arc<AtomicBool>,
}

pub struct WorkspaceEventReceiver {
    receiver: Receiver<WorkspaceCreated>,
    rescan: Arc<AtomicBool>,
}

pub fn workspace_event_channel(
    capacity: usize,
) -> io::Result<(WorkspaceEventSender, WorkspaceEventReceiver)> {
    if capacity == 0 || capacity > 4096 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let rescan = Arc::new(AtomicBool::new(false));
    Ok((
        WorkspaceEventSender {
            sender,
            rescan: rescan.clone(),
        },
        WorkspaceEventReceiver { receiver, rescan },
    ))
}

impl WorkspaceEventSender {
    /// Does not wait for the worker. Full/disconnected errors retain the hint
    /// in the error and request inventory reconciliation rather than admitting
    /// a native root from an incomplete event stream.
    pub fn try_publish(
        &self,
        event: WorkspaceCreated,
    ) -> Result<(), TrySendError<WorkspaceCreated>> {
        self.sender.try_send(event).inspect_err(|_| {
            self.rescan.store(true, Ordering::Release);
        })
    }
    pub fn rescan_required(&self) -> bool {
        self.rescan.load(Ordering::Acquire)
    }
}

impl WorkspaceEventReceiver {
    pub fn try_recv(&self) -> Result<WorkspaceCreated, TryRecvError> {
        self.receiver.try_recv()
    }
    /// Intended only for the independent worker, never a FUSE callback.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<WorkspaceCreated, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
    pub fn rescan_required(&self) -> bool {
        self.rescan.load(Ordering::Acquire)
    }
    /// Clear BEFORE scanning current authoritative inventory. Overflow racing
    /// with that scan re-arms the flag and requires a subsequent complete scan.
    /// If the scan fails, the worker must retain/retry its own outstanding scan.
    pub fn begin_rescan(&self) -> bool {
        self.rescan.swap(false, Ordering::AcqRel)
    }
}
