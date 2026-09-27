#[cfg(any(feature = "ownerfs", feature = "blobfs"))]
use afs::node::vfs::{
    Backend,
    types::{BackendInode, RequestContext},
};
use afs::node::vfs::{Namespace, Vfs};
use afs_error::ErrorKind;

#[test]
fn runtime_selection_exposes_only_enabled_namespaces() {
    #[cfg(feature = "ownerfs")]
    {
        let registry = afs_metrics::registry();
        let vfs = Vfs::new(true, false, registry.clone()).unwrap();
        assert_eq!(vfs.namespaces(), vec![Namespace::OwnerFs]);
    }

    #[cfg(feature = "blobfs")]
    {
        let registry = afs_metrics::registry();
        let vfs = Vfs::new(false, true, registry).unwrap();
        assert_eq!(vfs.namespaces(), vec![Namespace::BlobFs]);
    }
}

#[test]
fn empty_runtime_selection_is_rejected() {
    let err = Vfs::new(false, false, afs_metrics::registry()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn namespace_names_parse_strictly() {
    assert_eq!(Namespace::parse("ownerfs").unwrap(), Namespace::OwnerFs);
    assert_eq!(Namespace::parse("blobfs").unwrap(), Namespace::BlobFs);
    assert_eq!(
        Namespace::parse("workspace").unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn create_dispatches_to_selected_backend_and_reports_unsupported() {
    #[cfg(feature = "ownerfs")]
    {
        let registry = afs_metrics::registry();
        let vfs = Vfs::new(true, false, registry.clone()).unwrap();

        let err = vfs
            .create_file(Namespace::OwnerFs, "hello.txt")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unimplemented);

        let text = afs_metrics::encode_text(&registry).unwrap();
        assert!(text.contains(
        "afs_vfs_backend_operations_total{namespace=\"ownerfs\",operation=\"create\",result=\"unsupported\"} 1"
    ));
    }
}

#[test]
fn disabled_namespace_is_absent_at_runtime() {
    #[cfg(feature = "ownerfs")]
    {
        let vfs = Vfs::new(true, false, afs_metrics::registry()).unwrap();
        let err = vfs.create_file(Namespace::BlobFs, "hello.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }
}

#[cfg(any(feature = "ownerfs", feature = "blobfs"))]
#[test]
fn new_file_contract_does_not_fake_success_before_backend_implementation() {
    let context = RequestContext {
        uid: 1000,
        gid: 1000,
        pid: 42,
        umask: 0o022,
    };
    #[cfg(feature = "ownerfs")]
    {
        let backend = afs::node::vfs::ownerfs::OwnerFs::new();
        let root = BackendInode {
            namespace: Namespace::OwnerFs,
            value: 1,
        };
        assert_eq!(
            backend
                .lookup(&context, root, std::ffi::OsStr::new("file"))
                .unwrap_err()
                .kind(),
            ErrorKind::Unimplemented
        );
        assert_eq!(
            backend
                .create(&context, root, std::ffi::OsStr::new("file"), 0o644, 0)
                .unwrap_err()
                .kind(),
            ErrorKind::Unimplemented
        );
    }
    #[cfg(feature = "blobfs")]
    {
        let backend = afs::node::vfs::blobfs::BlobFs::new();
        let root = BackendInode {
            namespace: Namespace::BlobFs,
            value: 1,
        };
        assert_eq!(
            backend
                .lookup(&context, root, std::ffi::OsStr::new("draft"))
                .unwrap_err()
                .kind(),
            ErrorKind::Unimplemented
        );
        assert_eq!(
            backend
                .create(&context, root, std::ffi::OsStr::new("draft"), 0o644, 0)
                .unwrap_err()
                .kind(),
            ErrorKind::Unimplemented
        );
    }
}
