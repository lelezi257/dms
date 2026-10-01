
    #[test]
    fn diagnostic_cold_corrupted_local_copy_uses_healthy_peer() {
        diagnostic_corrupted_local_copy(false);
    }

    #[test]
    fn diagnostic_warm_corrupted_local_copy_uses_healthy_peer() {
        diagnostic_corrupted_local_copy(true);
    }

    fn diagnostic_corrupted_local_copy(warm: bool) {
        let temp = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalChunkStore::open(temp.path(), "node-a").unwrap());
        let mut builder = ChunkBuilder::default();
        builder.replace(b"abcdef".to_vec());
        let staged = builder.stage(OperationId::new("diagnostic"));
        let chunk_id = staged.chunk.id.clone();
        local.put(staged).unwrap();
        let sources = Arc::new(StaticSources::new(DfsChunkSourcesReply {
            revision: 1,
            chunks: vec![ChunkSources {
                chunk_id: chunk_id.clone(),
                sources: vec![source("healthy-b", &chunk_id, "node-b")],
            }],
        }));
        let engine = DfsReadEngine::new(
            crate::dfs::NamespaceId::new("default"),
            "node-a".into(), local, sources.clone(),
            Arc::new(StaticTransfer { bytes: b"abcdef".to_vec(), fail_first: Mutex::new(false) }),
            DfsReadConfig::default(),
        );
        let batch = ReadBatch {
            file_version_id: Some(FileVersionId::new("version")),
            layout_root_id: LayoutRootId::new("layout"),
            ops: vec![ChunkReadOp { chunk_id: chunk_id.clone(), chunk_offset: 0, length: 6, output_offset: 0 }],
        };
        let mut out = [0; 6];
        if warm {
            engine.read_batch(&batch, &mut out).unwrap();
            assert_eq!(&out, b"abcdef");
            assert_eq!(*sources.calls.lock().unwrap(), 0);
        }
        let path = temp.path().join("chunks").join(&chunk_id.0);
        std::fs::write(&path, b"abXdef").unwrap();
        std::fs::File::open(&path).unwrap().sync_all().unwrap();
        out.fill(9);
        engine.read_batch(&batch, &mut out).unwrap();
        assert_eq!(&out, b"abcdef", "corrupted cached local bytes must not reach the caller");
        assert_eq!(*sources.calls.lock().unwrap(), 1);
    }
