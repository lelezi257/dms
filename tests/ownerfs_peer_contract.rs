use std::{
    ffi::OsStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use afs::node::{
    rpc::{
        data::owner::{
            MtlsPeerAuthenticator, PeerAuthenticator, make_owner_files_handler,
            make_owner_files_server_with_handler,
        },
        peer::owner_files_client_from_channel,
    },
    storage::LocalFs,
    vfs::{
        Backend, Namespace,
        ownerfs::{
            OwnerFs,
            catalog::LocalRootRecord,
            remote::RemoteFiles,
            root::{
                PreparedRoot, RootGrant, RootId, RootLocation, RootManager, RootMeta,
                RootReservation, RootRight, root_id_from_name,
            },
        },
        types::{BackendInode, RenameFlags, RequestContext},
    },
};
use afs_protocol::node_data::{
    OwnerCreateRequest, OwnerLookupRequest, OwnerOpenRequest, OwnerReleaseRequest, RootAccess,
    owner_files_client::OwnerFilesClient,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    metadata::MetadataMap,
    transport::{
        Certificate, Channel, ClientTlsConfig, Endpoint, Identity, Server, ServerTlsConfig,
    },
};

const CA_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDDTCCAfWgAwIBAgIUG8Y1Uz4fkfWEstXKUj2ZSXzPO9IwDQYJKoZIhvcNAQEL
BQAwFjEUMBIGA1UEAwwLQUZTIFRlc3QgQ0EwHhcNMjYwOTI2MTMxNjA1WhcNMzYw
OTIzMTMxNjA1WjAWMRQwEgYDVQQDDAtBRlMgVGVzdCBDQTCCASIwDQYJKoZIhvcN
AQEBBQADggEPADCCAQoCggEBAJO1iyYRT/xJhc6WBY+1BZ3EEGwmxEgxEVzBr9Qj
USOHEphRpah7xDTLYRW7m8/+ks0RrNP3EbHTrBaWrIhtn6E9nIRTXjFPCTGxURZH
eDK9FnJyA7f39wnn+ntA9fkcCJbSoziTFK/awb5m+IM7i9yjSd+oNal5a3gX4CGU
0VzF//hWXCobKmNsisXn3Eq68p/Zk2ZBlnhcFcfWlqwpfpYLPI5vg2mfcB7/6fBj
uKLnEOQJ63rRAzVJnPJ+uvBcqVuhNcOWsvfKm++lg9zUcWxNj0NUcaoN+z/KVoww
vXNNMsf/qeoaKbD80dgA/7z58JzdCIcvj8HiDujS3fhYDEsCAwEAAaNTMFEwHQYD
VR0OBBYEFBGuiyvIVkf0gmmkYjM/EHUlIiMtMB8GA1UdIwQYMBaAFBGuiyvIVkf0
gmmkYjM/EHUlIiMtMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEB
ACxvpxjyCec9PkiEi3FEft7tYiDvCSIr/+8mneilcbPl2NBzdiTdOMnjei7dWwPh
pl9N+F29eB0lGn/OXTITbhj9VfFnvIytUHRLblPK48Lk9h346LBiw5kbxbDohZzy
vQnbKVj37QVbCDgyImtaNL6aqhTkCaktCDE5kkX8S4YQl2JYRElCEYkyPDrpGYRW
kCF+YI1RCdDvrlUY65w2sKK+vbbLBTT2B85NFzjnUBgTJcCcCLQjr77dmz1Wo1OY
H3fYD9LBiPmqt+qXwVTnc9SjoQUz/WXQ9MAuI6NZNc+oDfRkKv0WHPFnuUmqsqRl
21sgpGG7yJpiOKdZgErfCUU=
-----END CERTIFICATE-----"#;

const SERVER_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDKzCCAhOgAwIBAgIUExeNy5Q72Lhe/lXf4wFtZYqxdQ0wDQYJKoZIhvcNAQEL
BQAwFjEUMBIGA1UEAwwLQUZTIFRlc3QgQ0EwHhcNMjYwOTI2MTMxNjA1WhcNMzYw
OTIzMTMxNjA1WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQC85iyEx46fVzhAcGQg8QKxShbGFaQLycgfY78E4/49
qvosj1p0H9kMuaCvs3asSq6S8T/dgnxAxltjAY08DphAr+26dMW7f7FBH/VBej8o
YCgyamjAvh1v2MK65tawS1phIpfGEW91qZhbmy9LqxNH7/2rkKvxp4it4VmFCm6G
DcnfygUTrvDveaMMIOlEi93FM5KrJJRlXJwdiKGYDZQH9BaQPVL0BiCE53PMDuDJ
sZH/Na+iSuLzpGa8iVFitfQ85UqIEP/Z2OHES/rODGC9w3dLYISby63lV3XloAUV
Dciv5KkWol0y+ZcIQNNNXhO/gBE892zSUjw6EeI5yJPhAgMBAAGjczBxMBoGA1Ud
EQQTMBGCCWxvY2FsaG9zdIcEfwAAATATBgNVHSUEDDAKBggrBgEFBQcDATAdBgNV
HQ4EFgQUygnBDdtLqm09NySOQhkXhV4dXTowHwYDVR0jBBgwFoAUEa6LK8hWR/SC
aaRiMz8QdSUiIy0wDQYJKoZIhvcNAQELBQADggEBACh21WrMZLx+BGXohi5y1wlo
6kjEwkNBETOtt7DFpmmwg2AVDgfzDIhW35bFh80htWFhhzieBUe/vUNK6x3n4/+u
EHwd11II/iYoeKaYhBGyFNfgUQ40SdcdPqzH4I/ERd4UQXNl7XHYuSriXatjXxlF
USBfalMlnHROM3Im3TUpyDqmOjnSM4PRVNOlSNEuMUXwxbo+ueNxPKLNVVaA4n25
WDa62O0kJLqYAWTCi8hlZl3gIHYls+KNtmveZkbqJGjSwlbd/zoRVxfbt1L0oXtT
d7naClNAf66IdIZGAT27+B5tRow4tZ4Y/hzg2yo7Af1FeKLi0jtp+t9hgh77tV0=
-----END CERTIFICATE-----"#;

const SERVER_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC85iyEx46fVzhA
cGQg8QKxShbGFaQLycgfY78E4/49qvosj1p0H9kMuaCvs3asSq6S8T/dgnxAxltj
AY08DphAr+26dMW7f7FBH/VBej8oYCgyamjAvh1v2MK65tawS1phIpfGEW91qZhb
my9LqxNH7/2rkKvxp4it4VmFCm6GDcnfygUTrvDveaMMIOlEi93FM5KrJJRlXJwd
iKGYDZQH9BaQPVL0BiCE53PMDuDJsZH/Na+iSuLzpGa8iVFitfQ85UqIEP/Z2OHE
S/rODGC9w3dLYISby63lV3XloAUVDciv5KkWol0y+ZcIQNNNXhO/gBE892zSUjw6
EeI5yJPhAgMBAAECggEABl1vW11YCDUCi/HBG0wpA1f3ll7F+M1FD/MKafvwe1Sx
XeKtlNw6h1agymkLKFy2S4Lov9TYd/i6oM2Sv6xAqsI4JmPZAnnCgoeKT1Wr5ej/
GqJz0NJarpIsR866NDH4eSZsMgMwuFNNVshrWpylCvPigcu0w/ELRpLaQaEyohPH
joJ/C6YsYAcc3123mRz23CBvETYJzQYs9r7ci88iPrI85B7/5pZUSk5OoRlUi2sG
djagAFnBIdalKpxTcMuf2stzcC6CI2ouB0qY0odTZwiSVRP9+eo/EkB1CysXL2sl
9IATCqTC/ocKSc/1YK6g4JHns+z8tbTBIP8FeyRpGwKBgQD+BrCvN+jCB63teTAs
Mye3JMPBoPgChAnHR2hzGuMbsKBJdwtWLQSGCtMov81TRihWoHLgyWhuYN/UCXQJ
V9aSWF7Kd25Lze1mHnTmJHXhFMNFEtCu+h2QAVMvCmbY/ODBPs0VAXzqIW2My4aj
0G12tF0Bnf7KT/6mKLi3byI8HwKBgQC+Xe7L5p7FgrIOR76d8Rd8wbZpyHnhEhl7
HHz3juUTJlSncDfJplVI+Hyip6K/bIrI/Iwnz7w9bRkSGZOmNofzAtFgEQc2xSXd
kJQlKuTrL7EwF1SR6JOAB6k9T1qsRT7ytKr+oh+9fHd/Qd7NkIfq6tTRJ9wJWlyI
IJ7UTbsv/wKBgGBMEPabbzT+zERVyJk42zlmSn9AkkQB4eMVgtb/vlBk6J5w7m9A
qZJW0C2GaEPFOM1+DY6BS4FsX+11l/NixQi9T1HZbIp4CiLIMPB9qeIliNDKjSmH
z2Uj23DdtJdVZa5cLEpmQgBPo8PX87Zt8NErFobiahAvuw0qKrv++S9pAoGBAJDj
PVTDeiQpjQuBX3smfBHf/c4VX24GMI6a6CIjCAbDLbsildNMXazkMzg5Do1TN24x
iRrj6Ql3d5VnEhF3f5Fdm63aR/tPobo4yAhh1UmLSvinSR6kPV88dXrMYt6q9XYU
O/EBw9acXPbmU4Vxc4FAqilmhPo5ZCPXcAt1/fpRAoGAVNkKNIXqVu0jhxRFHSrp
Jx5X998LumoiRU38dmtkaKf+epfuYG4aZWVm7atA2Pwf8aaIBCXMdn5rnDDS4yFW
I9wR6akyXpNwmrFC5HdKh54WgXTZ/Qd8vgXQLhTXzizup9ksHOrLIYhsB+iXgCut
eFojDBvo55ePR948X9OHrfg=
-----END PRIVATE KEY-----"#;

const CLIENT_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDDDCCAfSgAwIBAgIUExeNy5Q72Lhe/lXf4wFtZYqxdQ4wDQYJKoZIhvcNAQEL
BQAwFjEUMBIGA1UEAwwLQUZTIFRlc3QgQ0EwHhcNMjYwOTI2MTMxNjA1WhcNMzYw
OTIzMTMxNjA1WjARMQ8wDQYDVQQDDAZub2RlLWIwggEiMA0GCSqGSIb3DQEBAQUA
A4IBDwAwggEKAoIBAQC//rRMEA/BAE181chtQetQA0TdLujKOPnYgITRznUpM16H
+c5H3eUtgaOEk0cVDSEFf77GDKzkPYDzICFeHtYZnzG3OjDlMaMCpaGTfYK3kWDE
VF0vwLgnPmbyf5giaoMEfRp+QP5/Tks3FlLZFeAD4KAhRfO2Wy2brIFUgc+K/V5D
YnXwgO9ptnywtpix1Er+0/zY0LsJzQE5QOunDPBh3vaaumVpyaoBvuDMZehVgCZF
td4Snb++zBwybWuF29yRPBQKUhO35YjY0NcQA11kKYLGQqt+w4PPRq43WJFA0NdQ
ITfBAHBVnGL8/nLf4JrfO5MRVrdE5J6nyMOUv2mXAgMBAAGjVzBVMBMGA1UdJQQM
MAoGCCsGAQUFBwMCMB0GA1UdDgQWBBS7kzepqT9ulwNaZdEkwDI1vl29sjAfBgNV
HSMEGDAWgBQRrosryFZH9IJppGIzPxB1JSIjLTANBgkqhkiG9w0BAQsFAAOCAQEA
BC6Zn7q/ctyOrylb92ejJBo/A1NmOt2OIR071kO0MV14bHIvCTVJoDEgZCT4XAQt
hKMUILtJjektvUvQb8Ah95qy7jXfd/AdxcDaQI3fYxoEvd7t6TxY09A6tL45Vbwq
pJRRMyo+kljkGoFsO4EZhGCV9iaw+ND1n1GiwS0fU9JihlwZ0rc+uLPd0tnp6Juj
fJlIAjrkaKVKOuNohUolCqIENn6ojD26l0eUrJ5kquJ+CXLLsH3kMuIDjAHU3E6j
bnJDCbTL/pQI2mqaZIxsDPJV6nAAW0IvH9AdI2B5w73LKxqWvKMFNy+eawlWyEst
RmIJ/4aMkpeOjj4nJcVloQ==
-----END CERTIFICATE-----"#;

const CLIENT_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC//rRMEA/BAE18
1chtQetQA0TdLujKOPnYgITRznUpM16H+c5H3eUtgaOEk0cVDSEFf77GDKzkPYDz
ICFeHtYZnzG3OjDlMaMCpaGTfYK3kWDEVF0vwLgnPmbyf5giaoMEfRp+QP5/Tks3
FlLZFeAD4KAhRfO2Wy2brIFUgc+K/V5DYnXwgO9ptnywtpix1Er+0/zY0LsJzQE5
QOunDPBh3vaaumVpyaoBvuDMZehVgCZFtd4Snb++zBwybWuF29yRPBQKUhO35YjY
0NcQA11kKYLGQqt+w4PPRq43WJFA0NdQITfBAHBVnGL8/nLf4JrfO5MRVrdE5J6n
yMOUv2mXAgMBAAECggEAGJwc3z0Vz97qj8xVXw/aikyI+LMJGta3y9UZcU09/lR7
0wElvFeIh089NwKr01p19576xKceSDlL/J4LOOXJ+snJlRtr5gz5QJ8beWzWoxIK
7c+EjFjvIfShPIc3aH3volUo3rMVDBtsj7iYUQQ5TTXvQKSXSzIfw/sWLs9e24lK
hvCFrXT8rsSollJ1L2UKaW/2VtUvqt1HJV+oCbxN7snDy81Dc9YtOHWepGAcfe4r
tUJzsteBnUbTDEWXAaRYmiW+bA69PPlVUd9AwROaZxbcAenl3g75MVRZihc7TbOc
DVT1ycwuPq6z+fcyMWIkh4P32oUqTkNMtwFkJxeuPQKBgQDzDcSU1Cbt6dHINbWB
mCCkYPBFA/u0GikEd4+ChoMiHILSx89xk//1Wgp2xubm3W+z8/EuKas3S51EZhO1
Mi9FO4vAeDsNu+kpEG+UUf7Ux3aRC6fS0DH53+X57coKE3m+cQV/jG8BPB7xZnra
Qi6O8jW/WNJ6fqYwwAgHA5uhIwKBgQDKOLVXTmaWZ6P/P/1AD/0hZhB2FtLZIa5g
94wY88qenxXwjHl1SYaRosm6Vjrx/JuK6hMJvU1FlJ2AMlv1dnMvatPTtMubA0Rc
ZNFjKC321QH2G+HIPggPrYlcbnfbuvURjzgmAwxV+kVGS0CX6aSExGaGtl4WurTD
EmYGWI3O/QKBgQDiZAybZBDuwkA48G4kTAL7mZ+zaUZmN4fFNWhi98/lUhE5LAw5
itV7P2dHw3UHzXJid/JKQV3Nn4zZTQtGV3xYTGKb4GGBJWrEaR7FVKq8nx39dJHZ
dztVuAuKhMcQI5vem3+3kqNCzzEzQXVlHwgm9czCcoV6u8Uo23WesumfaQKBgEMX
I1rO4Qw/YFJ7+Vp6s4GUKhvzoIp3OTJkjq9smqmboBzJjjZSaXoB5ymSGEZWh4hD
9oMBshRGpSZ2DrpWTQrLR3HyhqZsJA7/R9S87Nr6eocbYwIbSnNhILRw1gUpdssX
mApMcphHyxnyN4Du/C0sN9Ozx22FDhm2DfFHCe1FAoGBAJBAkyS0KGaYgs/8rAbL
cVSIL0zATu8Kv73EX7dHBzf5nZxAlQQ2nzNzjxv3Z8AlAz5/dztq2PSmcy5Tarpt
w1TlUs4iPMwsEQNBl8pRelqXHu/aUJ++fJ1Ax2HnziEWJO6q24YyyBafhHYyte52
zNEGIq4pivkTMT3xJGFmgGqs
-----END PRIVATE KEY-----"#;

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

#[derive(Clone)]
struct RequireMtlsNodeB;

impl PeerAuthenticator for RequireMtlsNodeB {
    fn authenticate(
        &self,
        _metadata: &MetadataMap,
        _remote_addr: Option<std::net::SocketAddr>,
        peer_cert_der: Option<&[u8]>,
    ) -> afs_error::Result<String> {
        if peer_cert_der.is_none() {
            return Err(afs_error::Error::coded(
                afs_error::NODE_OWNER_INVALID_GRANT,
                "test OwnerFiles peer must present mTLS certificate",
            ));
        }
        Ok("node-b".to_owned())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mtls_ownerfiles_grpc_roundtrip_uses_real_ownerfs_backend() {
    let temp = tempfile::tempdir().expect("tempdir");
    let disk = Arc::new(LocalFs::open(temp.path()).expect("localfs"));
    let meta = Arc::new(ContractMeta::default());
    let roots = Arc::new(RootManager::new(
        "node-a".to_owned(),
        "session-a".to_owned(),
        meta.clone(),
        disk.clone(),
    ));
    let fs = OwnerFs::new_local(roots, disk);
    let ctx = RequestContext {
        uid: 1000,
        gid: 1000,
        pid: 42,
        umask: 0,
    };
    let owner_root = BackendInode {
        namespace: Namespace::OwnerFs,
        value: 1,
    };
    fs.mkdir(&ctx, owner_root, OsStr::new("job-42"), 0o755)
        .expect("create home root");
    let root_id = root_id_from_name(OsStr::new("job-42")).expect("root id");
    let grant = meta
        .acquire_root(&root_id, RootRight::Write)
        .expect("remote grant");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let endpoint = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let handler = make_owner_files_handler(fs.peer_executor().expect("peer executor"));
    let server_tls = ServerTlsConfig::new()
        .client_ca_root(Certificate::from_pem(CA_PEM))
        .identity(Identity::from_pem(SERVER_CERT_PEM, SERVER_KEY_PEM));
    let server = tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls)
            .expect("server tls")
            .add_service(make_owner_files_server_with_handler(
                handler,
                Arc::new(RequireMtlsNodeB),
            ))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("owner files server");
    });

    let client_tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(CA_PEM))
        .identity(Identity::from_pem(CLIENT_CERT_PEM, CLIENT_KEY_PEM))
        .domain_name("localhost");
    let channel: Channel = Endpoint::from_shared(endpoint)
        .expect("endpoint")
        .timeout(Duration::from_secs(5))
        .tls_config(client_tls)
        .expect("client tls")
        .connect()
        .await
        .expect("connect");
    let missing_parent_status = OwnerFilesClient::new(channel.clone())
        .lookup(OwnerLookupRequest {
            access: Some(root_access_for(&grant)),
            path: b"missing-parent.txt".to_vec(),
            expected_parent_identity: None,
        })
        .await
        .expect_err("OwnerFiles.Lookup without non-root expected_parent_identity must fail");
    assert!(
        missing_parent_status
            .message()
            .contains("OwnerLookupRequest missing expected_parent_identity"),
        "unexpected status for missing expected_parent_identity: {missing_parent_status:?}"
    );
    let missing_parent_status = OwnerFilesClient::new(channel.clone())
        .create(OwnerCreateRequest {
            access: Some(root_access_for(&grant)),
            path: b"missing-parent.txt".to_vec(),
            flags: libc::O_RDWR as u32,
            mode: 0o644,
            expected_parent: None,
        })
        .await
        .expect_err("OwnerFiles.Create without expected_parent must fail");
    assert!(
        missing_parent_status
            .message()
            .contains("OwnerCreateRequest missing expected_parent"),
        "unexpected status for missing expected_parent: {missing_parent_status:?}"
    );
    let raw_channel = channel.clone();
    let raw_grant = grant.clone();
    let client = std::thread::spawn(move || owner_files_client_from_channel(channel))
        .join()
        .expect("OwnerFiles client construction must not require a Tokio reactor");

    tokio::task::spawn_blocking(move || -> afs_error::Result<()> {
        let root_parent = client.lookup(&grant, OsStr::new(""), None)?.identity;
        let created = client.create(
            &grant,
            OsStr::new("log.txt"),
            libc::O_RDWR,
            0o644,
            &root_parent,
        )?;
        assert_eq!(client.write(&grant, &created.file, 0, b"AAAA")?, 4);

        // The test Meta intentionally grants a second process session for
        // node-b. A valid root grant still must not borrow the first session's
        // opened Home handle, even though the opaque numeric ID is known.
        let mut other_session = grant.clone();
        other_session.session_id = "session-b-restarted".to_owned();
        let error = client
            .write(&other_session, &created.file, 0, b"XXXX")
            .expect_err("another peer session must not borrow an open handle");
        assert_eq!(error.code(), afs_error::NODE_OWNER_STALE_HANDLE);

        let mut buf = [0_u8; 4];
        assert_eq!(client.read(&grant, &created.file, 0, &mut buf)?, 4);
        assert_eq!(&buf, b"AAAA");

        client.rename(
            &grant,
            OsStr::new("log.txt"),
            OsStr::new("renamed.txt"),
            Some(&created.entry.identity),
            None,
            &root_parent,
            &root_parent,
            RenameFlags(0),
        )?;
        client.unlink(
            &grant,
            OsStr::new("renamed.txt"),
            Some(&created.entry.identity),
            &root_parent,
        )?;

        let recreated = client.create(
            &grant,
            OsStr::new("renamed.txt"),
            libc::O_RDWR,
            0o644,
            &root_parent,
        )?;
        assert_eq!(client.write(&grant, &recreated.file, 0, b"BBBB")?, 4);

        let mut old = [0_u8; 4];
        assert_eq!(client.read(&grant, &created.file, 0, &mut old)?, 4);
        assert_eq!(&old, b"AAAA");
        let mut new = [0_u8; 4];
        assert_eq!(client.read(&grant, &recreated.file, 0, &mut new)?, 4);
        assert_eq!(&new, b"BBBB");

        client.release(&grant, created.file.clone())?;
        let error = client
            .read(&grant, &created.file, 0, &mut old)
            .expect_err("released remote handle must be stale");
        assert_eq!(error.code(), afs_error::NODE_OWNER_STALE_HANDLE);
        client.release(&grant, recreated.file)?;
        Ok(())
    })
    .await
    .expect("blocking remote ops")
    .expect("remote ops");

    // Prefetch is an OwnerFs policy, not a Proto adapter policy. A new
    // read-only OPEN observes the final contents; writable OPEN never embeds
    // bytes and must continue through the normal file handle path.
    let mut raw = OwnerFilesClient::new(raw_channel);
    for (flags, expected_prefetch) in [
        (libc::O_RDONLY as u32, Some(b"BBBB".to_vec())),
        (libc::O_RDWR as u32, None),
    ] {
        let opened = raw
            .open(OwnerOpenRequest {
                access: Some(root_access_for(&raw_grant)),
                path: b"renamed.txt".to_vec(),
                flags,
                mode: 0,
                expected_file_identity: None,
            })
            .await
            .expect("open for prefetch contract")
            .into_inner();
        assert_eq!(opened.prefetched_data, expected_prefetch);
        raw.release(OwnerReleaseRequest {
            access: Some(root_access_for(&raw_grant)),
            handle: opened.handle,
        })
        .await
        .expect("release raw handle");
    }

    assert_eq!(*meta.validate_calls.lock().unwrap(), 2);
    server.abort();
}

fn root_access_for(grant: &RootGrant) -> RootAccess {
    RootAccess {
        root_id: grant.id.0.clone(),
        root_epoch: grant.epoch,
        access_generation: grant.access_generation,
        holder_node_id: grant.holder_node_id.clone(),
        home_node_id: grant.home_node_id.clone(),
        session_id: grant.session_id.clone(),
        fencing_token: grant.fencing_token.clone(),
        home_session_id: grant.home_session_id.clone(),
    }
}

#[test]
fn mtls_authenticator_is_available_for_exact_der_mapping() {
    assert!(MtlsPeerAuthenticator::new(vec![("node-b".to_owned(), vec![1, 2, 3])]).is_ok());
}
