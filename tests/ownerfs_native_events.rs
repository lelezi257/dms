#![cfg(feature = "ownerfs")]
use afs::node::vfs::{
    ownerfs::{
        OwnerFs,
        native::{WorkspaceCreated, workspace_event_channel},
    },
    types::BackendInode,
};
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    sync::mpsc::{TryRecvError, TrySendError},
};

fn event(name: &[u8], value: u64) -> WorkspaceCreated {
    WorkspaceCreated {
        name: OsString::from_vec(name.to_vec()),
        inode: BackendInode { value },
    }
}

#[test]
fn workspace_event_capacity_is_bounded() {
    for capacity in [0, 4097, usize::MAX] {
        assert!(workspace_event_channel(capacity).is_err());
    }
    assert!(workspace_event_channel(1).is_ok());
    assert!(workspace_event_channel(4096).is_ok());
}

#[test]
fn workspace_event_preserves_non_utf8_component_and_inode() {
    let (sender, receiver) = workspace_event_channel(1).unwrap();
    let expected = event(b"agent-\xff", 72);
    sender.try_publish(expected.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), expected);
    assert!(!receiver.rescan_required());
}

#[test]
fn full_queue_requires_rescan_without_waiting_or_overwriting() {
    let (sender, receiver) = workspace_event_channel(1).unwrap();
    sender.try_publish(event(b"first", 1)).unwrap();
    let rejected = event(b"second", 2);
    assert!(
        matches!(sender.try_publish(rejected.clone()), Err(TrySendError::Full(e)) if e == rejected)
    );
    assert!(receiver.rescan_required());
    assert_eq!(receiver.try_recv().unwrap(), event(b"first", 1));
    assert!(receiver.begin_rescan());
    assert!(!receiver.rescan_required());
    sender.try_publish(event(b"third", 3)).unwrap();
    assert!(sender.try_publish(event(b"during-scan", 4)).is_err());
    assert!(receiver.rescan_required());
    assert_eq!(receiver.try_recv().unwrap(), event(b"third", 3));
    assert_eq!(receiver.try_recv().unwrap_err(), TryRecvError::Empty);
}

#[test]
fn receiver_loss_is_observable_without_waiting() {
    let (sender, receiver) = workspace_event_channel(1).unwrap();
    drop(receiver);
    assert!(matches!(
        sender.try_publish(event(b"one", 1)),
        Err(TrySendError::Disconnected(_))
    ));
    assert!(sender.rescan_required());
}

#[test]
fn ownerfs_workspace_event_sink_installs_only_once() {
    let ownerfs = OwnerFs::new();
    let (sender, _receiver) = workspace_event_channel(1).unwrap();
    ownerfs.register_workspace_events(sender).unwrap();
    let (other, _other_receiver) = workspace_event_channel(1).unwrap();
    assert_eq!(
        ownerfs
            .register_workspace_events(other)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EALREADY)
    );
}
