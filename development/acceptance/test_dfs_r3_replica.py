"""Regression fixtures for persisted receipts, live authority and local ACK floors."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import dfs_r3_replica as observer

NOW = 2000
NODES = ('dfs-a-r3', 'dfs-b-r3', 'dfs-c-r3')


def state(epochs=(1,)):
    entities = []
    def add(kind, value, key=None):
        entities.append([key or {kind:value.get('id',value.get('chunk_id','fixture'))},
                         {'revision':10,'entity':{kind:value}}])
    for node in NODES:
        add('NodeSession', dict(node_id=node,session_id=node+'-current',lease_epoch=2,
            expires_at_unix_ms=NOW+1000,storage_devices=[dict(device_id='local-0',device_epoch=7,catalog_revision=16)]),
            {'CurrentNodeSession':{'node_id':node}})
    extents=[]
    for i in range(16):
        digest={'algorithm':'Blake3','bytes':[i]*32};cid='blake3-'+('%02x'%i)*32+'-4194304'
        chunk=dict(id=cid,length=4*2**20,encoding='Raw',content_digest=digest);add('DfsChunk',chunk)
        ids=[]
        for node in NODES:
            for epoch in epochs:
                ident=node+':'+str(epoch)+':local-0:'+cid;ids.append(ident)
                add('DfsCopy',dict(id=ident,chunk_id=cid,role='DurableReplica',state='Ready',persisted_bytes=chunk['length'],verified_digest=digest,
                    location={'Node':dict(node_id=node,node_epoch=epoch,device_id='local-0',device_epoch=7,catalog_revision=8)}))
        add('DfsPlacement',dict(chunk_id=cid,copies=ids,desired_copies=3,health='Satisfied'))
        extents.append(dict(file_offset=i*4*2**20,length=4*2**20,chunk_offset=0,chunk_id=cid))
    add('DfsLayoutRoot',dict(id='layout',file_length=64*2**20,inline_extents=extents))
    add('DfsFileVersion',dict(id='version',length=64*2**20,layout_root='layout'))
    return dict(schema_version=1,revision=10,entities=entities)


def entities(value, kind):
    return [row[1]['entity'][kind] for row in value['entities'] if kind in row[1]['entity']]


class ReceiptAuthorityTests(unittest.TestCase):
    def manifest(self,value):
        meta=dict(dir='/fixture',version=10,snapshot_frames=0,wal_frames=10,payload=value)
        with patch.object(observer,'load_meta_state',return_value=meta),patch('time.time_ns',return_value=NOW*10**6):
            return observer.build_manifest('/fixture',observer.EXPECTED_SHA256)

    def test_six_historical_rows_count_three_serving_nodes_without_mutation(self):
        value=state((1,2));before=copy.deepcopy(value);manifest=self.manifest(value)
        self.assertEqual(value,before)
        for chunk in manifest['chunks']:
            self.assertEqual(len(chunk['ready_durable_copies']),3)
            self.assertEqual({v['location']['node_id'] for v in chunk['ready_durable_copies']},set(NODES))
            self.assertEqual(len(chunk['historical_receipts']),6)

    def test_old_epoch_only_receipts_project_current_authority(self):
        value=state();manifest=self.manifest(value)
        for c in manifest['chunks']:
            for chosen in c['ready_durable_copies']:
                self.assertEqual(chosen['location']['node_epoch'],1)
                self.assertEqual(chosen['serving_location']['node_epoch'],2)

    def test_expired_and_equal_deadline_authorities_do_not_serve(self):
        for deadline in (NOW-1,NOW):
            value=state();entities(value,'NodeSession')[0]['expires_at_unix_ms']=deadline
            with self.subTest(deadline=deadline),self.assertRaises(ValueError):self.manifest(value)

    def test_missing_current_pointer_does_not_use_old_session_record(self):
        value=state();value['entities'][0][0]={'NodeSession':{'node_id':NODES[0],'session_id':'old'}}
        with self.assertRaises(ValueError):self.manifest(value)

    def test_wrong_session_node_device_and_cross_epoch_catalog_are_rejected(self):
        for key,val in [('node_id','foreign'),('lease_epoch',0),('device_id','other'),('device_epoch',8),('catalog_revision',7)]:
            value=state();session=entities(value,'NodeSession')[0]
            (session if key in ('node_id','lease_epoch') else session['storage_devices'][0])[key]=val
            with self.subTest(key=key),self.assertRaises(ValueError):self.manifest(value)

    def test_future_and_zero_receipt_epochs_do_not_serve(self):
        for epoch in (0,3):
            value=state((epoch,))
            with self.subTest(epoch=epoch),self.assertRaises(ValueError):self.manifest(value)

    def test_same_epoch_receipt_may_exceed_startup_catalog_floor(self):
        value=state((2,));entities(value,'NodeSession')[0]['storage_devices'][0]['catalog_revision']=0
        self.assertEqual(self.manifest(value)['status'],'PASS')

    def test_reject_non_durable_wrong_digest_length_and_chunk(self):
        for key,val in [('state','Missing'),('role','VerifiedCache'),('persisted_bytes',1),('chunk_id','wrong'),('verified_digest',None)]:
            value=state();entities(value,'DfsCopy')[0][key]=val
            with self.subTest(key=key),self.assertRaises(ValueError):self.manifest(value)

    def test_historical_record_of_dead_device_does_not_poison_eligible_current_copy(self):
        value=state((1,2));entities(value,'DfsCopy')[0]['location']['Node']['device_epoch']=6
        manifest=self.manifest(value);first=manifest['chunks'][0]
        self.assertEqual(len(first['ready_durable_copies']),3)
        self.assertEqual(len(first['historical_receipts']),6)
        self.assertTrue(any(not v['eligible'] for v in first['historical_receipts']))



class LocalCatalogTests(unittest.TestCase):
    def fixture(self):
        value=state();chunk=entities(value,'DfsChunk')[0];loc=entities(value,'DfsCopy')[0]['location']['Node']
        record=dict(state='Durable',encoding='Raw',chunk=chunk,stored_length=chunk['length'],stored_checksum=chunk['content_digest'],device_id='local-0',device_epoch=7,catalog_revision=7)
        return dict(records={chunk['id']:record},revisions=list(range(1,17)),device_epoch=7),dict(chunk=chunk),loc

    def test_ack_floor_is_covered_by_whole_catalog_not_equal_chunk_revision(self):
        catalog,chunk,loc=self.fixture();record,tip=observer.verify_catalog_record(catalog,chunk,loc)
        self.assertEqual((record['catalog_revision'],loc['catalog_revision'],tip),(7,8,16))

    def test_catalog_floor_and_device_rollback_are_rejected(self):
        catalog,chunk,loc=self.fixture()
        for bad in (dict(catalog,revisions=list(range(1,8))),dict(catalog,device_epoch=8)):
            with self.assertRaises(ValueError):observer.verify_catalog_record(bad,chunk,loc)
        for key,val in [('device_id','other'),('device_epoch',8),('catalog_revision',0),('catalog_revision',17)]:
            bad=copy.deepcopy(catalog);bad['records'][chunk['chunk']['id']][key]=val
            with self.subTest(key=key),self.assertRaises(ValueError):observer.verify_catalog_record(bad,chunk,loc)

    def test_local_non_durable_or_bad_metadata_is_rejected(self):
        catalog,chunk,loc=self.fixture()
        for key,val in [('state','Missing'),('encoding','other'),('stored_length',1),('stored_checksum',None),('chunk',{})]:
            bad=copy.deepcopy(catalog);bad['records'][chunk['chunk']['id']][key]=val
            with self.subTest(key=key),self.assertRaises(ValueError):observer.verify_catalog_record(bad,chunk,loc)

    def test_physical_content_and_symlink_identity_are_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);p=root/'chunk';p.write_bytes(observer.expected_chunk_bytes(0))
            proof=observer.verify_physical_chunk(p,dict(length=4*2**20),0);self.assertEqual(proof['bytes'],4*2**20)
            link=root/'alias';link.symlink_to(p)
            with self.assertRaises(ValueError):observer.verify_physical_chunk(link,dict(length=4*2**20),0)
            with p.open('r+b') as f:f.seek(100);f.write(b'!')
            with self.assertRaises(ValueError):observer.verify_physical_chunk(p,dict(length=4*2**20),0)

class CombineProvenanceTests(unittest.TestCase):
    def test_physical_proof_accepts_distinct_chunk_revision_and_ack_floor(self):
        chunk=dict(chunk_id='chunk',expected_sha256='sha')
        receipt=dict(copy_id='copy',location=dict(node_id='node',device_id='local-0',device_epoch=7,catalog_revision=8),serving_location=dict(node_epoch=2))
        item=dict(node_id='node',copy_id='copy',chunk_id='chunk',sha256='sha',bytes=4*2**20,receipt_catalog_floor=8,catalog_tip_revision=16,catalog_revision=7,device_epoch=7,device_id='local-0',serving_node_epoch=2)
        observer.verify_physical_receipt(item,chunk,receipt)
        for key,val in [('node_id','other'),('copy_id','other'),('chunk_id','other'),('sha256','wrong'),('bytes',1),('receipt_catalog_floor',7),('catalog_tip_revision',7),('catalog_revision',17),('device_epoch',8),('device_id','other'),('serving_node_epoch',1)]:
            bad=dict(item);bad[key]=val
            with self.subTest(key=key),self.assertRaises(ValueError):observer.verify_physical_receipt(bad,chunk,receipt)

    def test_manifest_requires_fixed_candidate_and_clocked_authority(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'manifest.json';base=dict(status='PASS',mode='meta',dataset=observer.DATASET,file_sha256=observer.EXPECTED_SHA256,source_commit=observer.SOURCE_COMMIT,meta_elf_sha256=observer.META_ELF_SHA256,node_elf_sha256=observer.NODE_ELF_SHA256,authority_at_unix_ms=NOW)
            p.write_text(json.dumps(base));observer.load_manifest(p)
            for key,val in [('source_commit','other'),('meta_elf_sha256','other'),('node_elf_sha256','other'),('mode','node'),('authority_at_unix_ms',0)]:
                bad=dict(base);bad[key]=val;p.write_text(json.dumps(bad))
                with self.subTest(key=key),self.assertRaises(ValueError):observer.load_manifest(p)

if __name__=='__main__':unittest.main()
