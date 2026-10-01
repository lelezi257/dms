#!/usr/bin/env python3
"""Preserve the pre-repair cataloged-copy failure on a separate Linux snapshot."""
import hashlib,json,os,pathlib,platform,shutil,subprocess,time
assert platform.system()=='Linux'
b=pathlib.Path('/home/lzc.guest/afs-build');root=b/'work/corrupt-retry-original-v61';out=b/'probes/corrupt-retry-original-v61'
out.mkdir(exist_ok=False);shutil.copytree(b/'work/root-merged-v61-local-r1',root,ignore=shutil.ignore_patterns('target','.git','._*'))
p=root/'src/node/chunk.rs';original=p.read_bytes();fixture=r'''
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
'''
marker='    #[test]\n    fn pinned_reader_verifies_partial_ranges_and_rejects_damage_outside_them()';assert original.decode().count(marker)==1
p.write_text(original.decode().replace(marker,fixture+'\n'+marker));(out/'fixture.rs').write_text(fixture)
(out/'inputs.json').write_text(json.dumps({'baseline_chunk_sha256':hashlib.sha256(original).hexdigest(),'diagnostic_chunk_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'scope':'only test fixture injected in prior v61 reader snapshot; product unchanged'},indent=2)+'\n')
env=os.environ.copy();env.update(PATH='/home/lzc.guest/.cargo/bin:'+env['PATH'],CARGO_TARGET_DIR=str(b/'target'),CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0')
a=['timeout','180','cargo','test','--all-features','--lib','diagnostic_cataloged_corrupt_copy_retry_restores_bytes','--','--nocapture'];begin=time.monotonic()
with (out/'original.log').open('w') as log:rc=subprocess.run(a,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
report={'status':'FAIL','diagnostic_harness':'PASS' if rc==101 else 'FAIL','returncode':rc,'argv':a,'seconds':time.monotonic()-begin,'scope':'original cataloged bad-copy retransmission fails digest verification; not a fixed-product or formal gate'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True);assert rc==101
