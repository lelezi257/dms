use afs::{meta::Meta, runtime::Observability};
#[test]
fn both_edges_can_reuse_the_same_meta_state() {
    let meta = Meta {
        id: "meta-test".into(),
        observability: Observability::new().unwrap(),
    };
    assert_eq!(meta.ping("node-a").unwrap(), "pong from meta-test");
    assert!(meta.ping(&"x".repeat(129)).is_err());
    let text = afs_metrics::encode_text(&meta.observability.registry).unwrap();
    assert!(text.contains("result=\"ok\"} 1"));
    assert!(text.contains("result=\"error\"} 1"));
}
