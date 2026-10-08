import copy
from pathlib import Path
import unittest

from dfs_r3_recovery import RetainedFixture, verify_closed, verify_restart


def closed():
    root = Path('/mnt/lima-afsctlstate/afs-delivery/owned')
    lifecycle = str(root/'run/meta.lifecycle.one')
    child = dict(pid='20',supervisor_pid='19',start_ticks='10',boot_id='boot',
                 exe=str(root/'prefix/bin/afs-meta'),config=str(root/'etc/meta.toml'),lifecycle=lifecycle)
    wait = dict(path=lifecycle,child=child,ready={'supervisor_pid':'19'},exit=dict(child,exit_code='0'))
    value = dict(status='PASS',mode='closed',role='ctl',actual_wait=wait,
                 frozen=dict(root=str(root),executable=child['exe'],executable_sha256='a'*64))
    return root, value


class RecoveryGuards(unittest.TestCase):
    def verify(self,value,records=None,gone=True):
        root,_=closed()
        verify_closed(value,role='ctl',root=root,binary_sha='a'*64,boot_id='boot',
                      records=records or value['actual_wait'],gone=gone)

    def test_prior_exact_closed_incarnation_is_accepted(self):
        _,value=closed();self.verify(value)

    def test_wait0_and_dead_processes_are_required(self):
        _,value=closed()
        for code in ('1','124','137','143'):
            bad=copy.deepcopy(value);bad['actual_wait']['exit']['exit_code']=code
            with self.subTest(code=code),self.assertRaises(RuntimeError):self.verify(bad)
        with self.assertRaises(RuntimeError):self.verify(value,gone=False)
        bad=copy.deepcopy(value['actual_wait']);bad['child']['start_ticks']='11'
        with self.assertRaises(RuntimeError):self.verify(value,records=bad)

    def test_foreign_role_binary_boot_path_and_duplicate_pid_are_rejected(self):
        _,value=closed()
        for field,replacement in [('role','a'),('mode','capture'),('status','FAIL')]:
            bad=copy.deepcopy(value);bad[field]=replacement
            with self.subTest(field=field),self.assertRaises(RuntimeError):self.verify(bad)
        for key,val in [('boot_id','other'),('exe','/foreign/afs-meta'),('config','/foreign/meta.toml'),
                        ('supervisor_pid','20'),('start_ticks','0')]:
            bad=copy.deepcopy(value);bad['actual_wait']['child'][key]=val
            with self.subTest(key=key),self.assertRaises(RuntimeError):self.verify(bad)
        bad=copy.deepcopy(value);bad['actual_wait']['path']='/foreign/run/meta.lifecycle.one'
        with self.assertRaises(RuntimeError):self.verify(bad)

    def test_retained_admission_never_allows_policy_patching(self):
        fixture=RetainedFixture('a','owned',closed=Path('/unused'))
        with self.assertRaisesRegex(RuntimeError,'read-only'):fixture.patch()

    def transition(self):
        _,first=closed()
        before=dict(status='PASS',mode='capture',frozen=first['frozen'],child=first['actual_wait']['child'],
                    initial_sha256='initial',lifecycle=first['actual_wait']['path'])
        after=copy.deepcopy(before);after['child'].update(pid='30',supervisor_pid='29',start_ticks='20',
            lifecycle=before['lifecycle'].replace('one','two'));after['lifecycle']=after['child']['lifecycle']
        nodes={r:dict(pid=100+i,mount_id=10+i,uds_inode=20+i) for i,r in enumerate('abc')}
        return before,after,first,nodes

    def test_successor_accepts_saved_first_receipt_without_old_directory_retention(self):
        before,after,first,nodes=self.transition()
        verify_restart(before,after,first,nodes,nodes)

    def test_restart_refuses_reused_meta_changed_node_mount_config_or_first_receipt(self):
        before,after,first,nodes=self.transition()
        with self.assertRaises(RuntimeError):verify_restart(before,before,first,nodes,nodes)
        for key in ('pid','mount_id','uds_inode'):
            changed=copy.deepcopy(nodes);changed['b'][key]+=1
            with self.subTest(key=key),self.assertRaises(RuntimeError):verify_restart(before,after,first,nodes,changed)
        changed=copy.deepcopy(after);changed['frozen']['executable_sha256']='b'*64
        with self.assertRaises(RuntimeError):verify_restart(before,changed,first,nodes,nodes)
        changed=copy.deepcopy(first);changed['actual_wait']['exit']['exit_code']='1'
        with self.assertRaises(RuntimeError):verify_restart(before,after,changed,nodes,nodes)
        with self.assertRaises(RuntimeError):verify_restart(before,after,first,nodes,{'a':nodes['a']})


if __name__=='__main__':unittest.main()
