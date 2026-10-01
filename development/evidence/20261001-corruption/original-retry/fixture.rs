
    #[test]
    fn diagnostic_cataloged_corrupt_copy_retry_restores_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalChunkStore::open(temp.path(), "node-a").unwrap();
        let staged = StagedChunk::new(OperationId::new("corrupt-retry"), b"abcdef".to_vec());
        let first = store.put(staged.clone()).unwrap();
        fs::write(temp.path().join("chunks").join(&staged.chunk.id.0), b"abXdef").unwrap();
        let repaired = store.put(staged.clone()).unwrap();
        assert!(repaired.durable_acks[0].catalog_revision > first.durable_acks[0].catalog_revision);
        let mut out = [9; 6];
        store.read_at(&staged.chunk.id, 0, &mut out).unwrap();
        assert_eq!(&out, b"abcdef");
    }
