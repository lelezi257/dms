use super::*;
use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::node::vfs::ownerfs::root::{
    OwnerRootInventory, PresentedRootAccess, RootLocation, RootMeta, RootReservation,
};

struct RejectRemoteFactory;

impl RemoteFilesFactory for RejectRemoteFactory {
    fn connect(&self, _: &str) -> Result<Arc<dyn remote::RemoteFiles>> {
        Err(Error::coded(
            afs_error::NODE_VFS_UNIMPLEMENTED,
            "native home test does not connect remote files",
        ))
    }
}

struct NativeHomeMeta {
    node_id: String,
    session_id: String,
    next_epoch: AtomicU64,
    active: Mutex<HashMap<RootId, RootLocation>>,
}

impl NativeHomeMeta {
    fn new() -> Self {
        Self {
            node_id: "node-a".into(),
            session_id: "session-a".into(),
            next_epoch: AtomicU64::new(1),
            active: Mutex::new(HashMap::new()),
        }
    }
}

impl RootMeta for NativeHomeMeta {
    fn reserve_root(&self, id: &RootId, create_intent_id: &str) -> Result<RootReservation> {
        let epoch = self.next_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(RootReservation {
            id: id.clone(),
            epoch,
            home_node_id: self.node_id.clone(),
            session_id: self.session_id.clone(),
            create_intent_id: create_intent_id.to_owned(),
            prepare_token: format!("prepare-{epoch}"),
        })
    }

    fn activate_root(&self, prepared: &root::PreparedRoot) -> Result<RootGrant> {
        self.active.lock().unwrap().insert(
            prepared.reservation().id.clone(),
            RootLocation {
                id: prepared.reservation().id.clone(),
                epoch: prepared.reservation().epoch,
                home_node_id: self.node_id.clone(),
                home_session_id: self.session_id.clone(),
            },
        );
        Ok(RootGrant {
            id: prepared.reservation().id.clone(),
            epoch: prepared.reservation().epoch,
            home_node_id: self.node_id.clone(),
            home_session_id: self.session_id.clone(),
            holder_node_id: self.node_id.clone(),
            session_id: self.session_id.clone(),
            access_generation: prepared.reservation().epoch,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
            fencing_token: format!("fence-{}", prepared.reservation().epoch),
        })
    }

    fn abort_root(&self, _: &RootReservation) -> Result<()> {
        Ok(())
    }

    fn lookup_root(&self, id: &RootId) -> Result<Option<RootLocation>> {
        Ok(self.active.lock().unwrap().get(id).cloned())
    }

    fn list_owner_roots(&self, _: &str) -> Result<OwnerRootInventory> {
        Ok(OwnerRootInventory {
            active: self.active.lock().unwrap().values().cloned().collect(),
            pending: Vec::new(),
        })
    }

    fn acquire_root(&self, _: &RootId, _: RootRight) -> Result<RootGrant> {
        Err(Error::coded(
            afs_error::NODE_OWNER_GRANT_UNAVAILABLE,
            "native home test does not acquire remote roots",
        ))
    }

    fn validate_root_access(
        &self,
        presented: &PresentedRootAccess,
        authenticated_peer_node_id: &str,
    ) -> Result<RootGrant> {
        Ok(RootGrant {
            id: presented.id.clone(),
            epoch: presented.epoch,
            home_node_id: presented.home_node_id.clone(),
            home_session_id: presented.home_session_id.clone(),
            holder_node_id: authenticated_peer_node_id.to_owned(),
            session_id: presented.session_id.clone(),
            access_generation: presented.access_generation,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
            fencing_token: presented.fencing_token.clone(),
        })
    }

    fn recover_root(
        &self,
        record: &catalog::LocalRootRecord,
        new_session_id: &str,
    ) -> Result<RootGrant> {
        Ok(RootGrant {
            id: record.id.clone(),
            epoch: record.epoch,
            home_node_id: self.node_id.clone(),
            home_session_id: new_session_id.to_owned(),
            holder_node_id: self.node_id.clone(),
            session_id: new_session_id.to_owned(),
            access_generation: record.epoch,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
            fencing_token: format!("recover-fence-{}", record.epoch),
        })
    }
}

pub(super) fn fixture(
    native_home_eligible: bool,
) -> (tempfile::TempDir, OwnerFs, RequestContext, Arc<LocalFs>) {
    let temp = tempfile::tempdir().unwrap();
    let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
    let meta = Arc::new(NativeHomeMeta::new());
    let roots = Arc::new(RootManager::new(
        "node-a".into(),
        "session-a".into(),
        meta,
        disk.clone(),
    ));
    let metadata = fs::metadata(temp.path()).unwrap();
    let ctx = RequestContext {
        uid: metadata.uid(),
        gid: metadata.gid(),
        pid: 42,
        umask: 0,
        supplementary_gids: Vec::new(),
    };
    let fs = if native_home_eligible {
        OwnerFs::new_local_native_home_for_tests(roots, disk.clone())
    } else {
        OwnerFs::new_local(roots, disk.clone())
    };
    (temp, fs, ctx, disk)
}

fn fixture_native_home_eligible_with_remote() -> (tempfile::TempDir, OwnerFs, RequestContext) {
    let temp = tempfile::tempdir().unwrap();
    let disk = Arc::new(LocalFs::open(temp.path()).unwrap());
    let meta = Arc::new(NativeHomeMeta::new());
    let roots = Arc::new(RootManager::new(
        "node-a".into(),
        "session-a".into(),
        meta,
        disk.clone(),
    ));
    let metadata = fs::metadata(temp.path()).unwrap();
    let ctx = RequestContext {
        uid: metadata.uid(),
        gid: metadata.gid(),
        pid: 42,
        umask: 0,
        supplementary_gids: Vec::new(),
    };
    let fs =
        OwnerFs::new_local_native_eligible_with_remote(roots, disk, Arc::new(RejectRemoteFactory));
    (temp, fs, ctx)
}

pub(super) fn mkdir_root(fs: &OwnerFs, ctx: &RequestContext, name: &str) -> Entry {
    Backend::mkdir(fs, ctx, fs.root_inode(), OsStr::new(name), 0o755).unwrap()
}

fn lookup_root(fs: &OwnerFs, ctx: &RequestContext, name: &str) -> Result<Entry> {
    Backend::lookup(fs, ctx, fs.root_inode(), OsStr::new(name))
}

#[test]
fn native_home_ordinary_instance_rejects_authority_and_keeps_private_cache() {
    let (_temp, fs, ctx, _disk) = fixture(false);
    let root = mkdir_root(&fs, &ctx, "ordinary");

    let error = match fs.native_home_export_for_current_namespace(OsStr::new("ordinary")) {
        Ok(_) => panic!("ordinary OwnerFs unexpectedly issued native Home authority"),
        Err(error) => error,
    };
    assert_eq!(error.code(), afs_error::NODE_OWNER_INVALID_GRANT);

    fs.with_fuse_cache_policy(root.inode, |ttl, private| {
        assert_eq!(ttl, Duration::from_secs(1));
        assert!(private);
    });
}

#[test]
fn native_home_eligible_instance_uses_zero_ttl_from_first_reply() {
    let (_temp, fs, ctx, _disk) = fixture(true);
    let root = mkdir_root(&fs, &ctx, "native");

    fs.with_fuse_cache_policy(root.inode, |ttl, private| {
        assert_eq!(ttl, Duration::ZERO);
        assert!(!private);
    });

    let authority: HomeExportAuthority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    authority.verify_current(&fs).unwrap();
}

#[test]
fn native_home_explicit_remote_constructor_is_eligible() {
    let (_temp, fs, ctx) = fixture_native_home_eligible_with_remote();
    let root = mkdir_root(&fs, &ctx, "native");

    fs.with_fuse_cache_policy(root.inode, |ttl, private| {
        assert_eq!(ttl, Duration::ZERO);
        assert!(!private);
    });

    fs.native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap()
        .verify_current(&fs)
        .unwrap();
}

#[test]
fn native_home_authority_exports_thread_namespace_and_cloned_source_descriptor() {
    let (_temp, fs, ctx, _disk) = fixture(true);
    mkdir_root(&fs, &ctx, "native");

    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    assert_eq!(authority.name(), OsStr::new("native"));
    assert_eq!(authority.grant().home_node_id, "node-a");
    assert_eq!(authority.grant().holder_node_id, "node-a");
    assert_eq!(authority.grant().home_session_id, "session-a");
    assert_eq!(authority.grant().session_id, "session-a");
    assert_eq!(authority.grant().access_generation, authority.grant().epoch);
    for right in [RootRight::Lookup, RootRight::Read, RootRight::Write] {
        assert!(authority.grant().rights.contains(&right));
    }

    let expected_namespace = fs::metadata("/proc/thread-self/ns/mnt").unwrap();
    assert_eq!(
        authority.namespace(),
        NativeHomeNamespaceIdentity {
            dev: expected_namespace.dev(),
            ino: expected_namespace.ino(),
        }
    );

    let first = authority.source_descriptor().unwrap();
    let second = authority.source_descriptor().unwrap();
    assert_ne!(first.as_raw_fd(), second.as_raw_fd());
    let metadata = first.metadata().unwrap();
    assert!(metadata.is_dir());
    assert_eq!(
        authority.source_identity(),
        NativeHomeDirectoryIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    );
    authority.verify_current(&fs).unwrap();
}

#[test]
fn native_home_authority_fails_closed_for_wrong_owner() {
    let (_left_temp, left, left_ctx, _left_disk) = fixture(true);
    mkdir_root(&left, &left_ctx, "native");
    let authority = left
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();

    let (_right_temp, right, right_ctx, _right_disk) = fixture(true);
    mkdir_root(&right, &right_ctx, "native");

    let error = authority.verify_current(&right).unwrap_err();
    assert_eq!(error.code(), afs_error::NODE_OWNER_INVALID_GRANT);
}

#[test]
fn native_home_authority_fails_closed_after_source_replacement() {
    let (temp, fs, ctx, _disk) = fixture(true);
    mkdir_root(&fs, &ctx, "native");
    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();

    let data_path = temp.path().join(authority.data_dir().as_path());
    let backup_path = temp.path().join("native-replaced-backup");
    fs::rename(&data_path, &backup_path).unwrap();
    fs::create_dir(&data_path).unwrap();

    let error = authority.verify_current(&fs).unwrap_err();
    assert_eq!(error.code(), afs_error::NODE_OWNER_INVALID_GRANT);
}

#[test]
fn native_home_authority_fails_closed_after_revoke_and_invalidate() {
    let (_temp, fs, ctx, _disk) = fixture(true);
    mkdir_root(&fs, &ctx, "native");
    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    let root_id = authority.root_id().clone();

    fs.require_local().unwrap().roots.revoke_root(&root_id);
    assert!(authority.verify_current(&fs).is_err());

    drop(authority);
    mkdir_root(&fs, &ctx, "fresh");
    let fresh = fs
        .native_home_export_for_current_namespace(OsStr::new("fresh"))
        .unwrap();
    fs.require_local().unwrap().roots.invalidate_all();
    assert!(fresh.verify_current(&fs).is_err());
}

#[test]
fn native_home_anchor_covers_only_root_metadata_until_authority_drops() {
    let (_temp, fs, ctx, _disk) = fixture(true);
    let root = mkdir_root(&fs, &ctx, "native");
    Backend::mkdir(&fs, &ctx, root.inode, OsStr::new("child"), 0o755).unwrap();
    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    let root_id = authority.root_id().clone();

    fs.require_local().unwrap().roots.revoke_root(&root_id);

    let anchored = lookup_root(&fs, &ctx, "native").unwrap();
    assert_eq!(anchored.inode, root.inode);
    Backend::getattr(&fs, &ctx, root.inode, None).unwrap();
    assert!(Backend::lookup(&fs, &ctx, root.inode, OsStr::new("child")).is_err());

    drop(authority);
    assert!(lookup_root(&fs, &ctx, "native").is_err());
}

#[test]
fn native_home_stale_full_grant_is_rejected() {
    let (_temp, fs, ctx, _disk) = fixture(true);
    mkdir_root(&fs, &ctx, "native");
    let authority = fs
        .native_home_export_for_current_namespace(OsStr::new("native"))
        .unwrap();
    let root_id = authority.root_id().clone();

    fs.require_local().unwrap().roots.revoke_root(&root_id);
    let error = authority.verify_current(&fs).unwrap_err();
    assert_eq!(error.code(), afs_error::NODE_OWNER_GRANT_UNAVAILABLE);
}

include!("native_home_fuse_fixture.rs");
