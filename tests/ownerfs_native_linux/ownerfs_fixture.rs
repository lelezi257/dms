//! Real OwnerFs with an in-process authoritative Meta fixture, not a P2P lane.
use afs::node::{
    storage::LocalFs,
    vfs::ownerfs::{
        OwnerFs,
        catalog::LocalRootRecord,
        root::{
            PreparedRoot, RootGrant, RootId, RootLocation, RootManager, RootMeta, RootReservation,
            RootRight,
        },
    },
};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct ContractMeta {
    active: Mutex<Option<RootGrant>>,
    validate_calls: Mutex<usize>,
}

impl ContractMeta {
    fn grant_for(&self, holder: &str, session: &str, right: RootRight) -> RootGrant {
        let active = self
            .active
            .lock()
            .unwrap()
            .clone()
            .expect("active root grant");
        RootGrant {
            id: active.id,
            epoch: active.epoch,
            home_node_id: active.home_node_id,
            home_session_id: active.home_session_id,
            holder_node_id: holder.to_owned(),
            session_id: session.to_owned(),
            access_generation: active.access_generation,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write, right],
            fencing_token: active.fencing_token,
        }
    }
}

impl RootMeta for ContractMeta {
    fn reserve_root(
        &self,
        id: &RootId,
        create_intent_id: &str,
    ) -> afs_error::Result<RootReservation> {
        Ok(RootReservation {
            id: id.clone(),
            epoch: 1,
            home_node_id: "node-a".to_owned(),
            session_id: "session-a".to_owned(),
            create_intent_id: create_intent_id.to_owned(),
            prepare_token: "prepare-token".to_owned(),
        })
    }

    fn activate_root(&self, prepared: &PreparedRoot) -> afs_error::Result<RootGrant> {
        let grant = RootGrant {
            id: prepared.reservation().id().clone(),
            epoch: prepared.reservation().epoch,
            home_node_id: "node-a".to_owned(),
            home_session_id: "session-a".to_owned(),
            holder_node_id: "node-a".to_owned(),
            session_id: "session-a".to_owned(),
            access_generation: 1,
            rights: vec![RootRight::Lookup, RootRight::Read, RootRight::Write],
            fencing_token: "fence-1".to_owned(),
        };
        *self.active.lock().unwrap() = Some(grant.clone());
        Ok(grant)
    }

    fn abort_root(&self, _reservation: &RootReservation) -> afs_error::Result<()> {
        Ok(())
    }

    fn lookup_root(&self, id: &RootId) -> afs_error::Result<Option<RootLocation>> {
        Ok(self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .filter(|grant| &grant.id == id)
            .map(|grant| RootLocation {
                id: grant.id.clone(),
                epoch: grant.epoch,
                home_node_id: grant.home_node_id.clone(),
                home_session_id: grant.home_session_id.clone(),
            }))
    }

    fn list_owner_roots(
        &self,
        home_node_id: &str,
    ) -> afs_error::Result<afs::node::vfs::ownerfs::root::OwnerRootInventory> {
        Ok(afs::node::vfs::ownerfs::root::OwnerRootInventory {
            active: self
                .active
                .lock()
                .unwrap()
                .as_ref()
                .filter(|grant| grant.home_node_id == home_node_id)
                .map(|grant| RootLocation {
                    id: grant.id.clone(),
                    epoch: grant.epoch,
                    home_node_id: grant.home_node_id.clone(),
                    home_session_id: grant.home_session_id.clone(),
                })
                .into_iter()
                .collect(),
            pending: Vec::new(),
        })
    }

    fn acquire_root(&self, id: &RootId, right: RootRight) -> afs_error::Result<RootGrant> {
        let grant = self.grant_for("node-b", "session-b", right);
        assert_eq!(&grant.id, id);
        Ok(grant)
    }

    fn validate_root_access(
        &self,
        presented: &afs::node::vfs::ownerfs::root::PresentedRootAccess,
        authenticated_peer_node_id: &str,
    ) -> afs_error::Result<RootGrant> {
        assert_eq!(authenticated_peer_node_id, "node-b");
        assert_eq!(presented.holder_node_id, "node-b");
        *self.validate_calls.lock().unwrap() += 1;
        Ok(self.grant_for(
            &presented.holder_node_id,
            &presented.session_id,
            RootRight::Write,
        ))
    }

    fn recover_root(
        &self,
        _record: &LocalRootRecord,
        _new_session_id: &str,
    ) -> afs_error::Result<RootGrant> {
        Err(afs_error::Error::coded(
            afs_error::META_STORE_UNIMPLEMENTED,
            "recovery is outside this peer contract test",
        ))
    }
}

pub fn ownerfs_fixture(data: &std::path::Path) -> (Arc<LocalFs>, Arc<RootManager>, Arc<OwnerFs>) {
    let disk = Arc::new(LocalFs::open(data).unwrap());
    let roots = Arc::new(RootManager::new(
        "node-a".into(),
        "session-a".into(),
        Arc::new(ContractMeta::default()),
        disk.clone(),
    ));
    let fs = Arc::new(OwnerFs::new_local(roots.clone(), disk.clone()));
    (disk, roots, fs)
}
