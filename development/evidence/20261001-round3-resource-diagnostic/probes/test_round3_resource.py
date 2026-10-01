import importlib.util,os,pathlib,subprocess,sys,tempfile,unittest
from unittest.mock import patch
assert sys.platform=='linux'
spec=importlib.util.spec_from_file_location('resource_probe',pathlib.Path(__file__).with_name('round3-resource-probe.py'));probe=importlib.util.module_from_spec(spec);sys.modules[spec.name]=probe;spec.loader.exec_module(probe)
class ResourceTests(unittest.TestCase):
 def test_live_self(self):
  with tempfile.TemporaryDirectory() as d:
   p=pathlib.Path(d)/'pid';p.write_text(str(os.getpid()));r=probe.collect_process(probe.parse_pid_file('self='+str(p)))
   self.assertTrue(r['identity']['stable_pre_post']);self.assertGreater(r['fd_count']['count'],0);self.assertGreater(r['task_count']['count'],0);self.assertIn('VmRSS',r['status_fields']);self.assertEqual(r['io']['status'],'OBSERVED')
 def test_role_rejection(self):
  for value in ['x','=/missing','a/b=/missing']:
   with self.assertRaises(probe.ProbeError):probe.parse_pid_file(value)
 def test_bad_pid(self):
  with tempfile.TemporaryDirectory() as d:
   p=pathlib.Path(d)/'pid'
   for value in ['0','-1','invalid']:
    p.write_text(value)
    with self.assertRaises(probe.ProbeError):probe.parse_pid_file('x='+str(p))
 def test_bad_stat(self):
  for value in ['','0 no-parentheses','0 (name) S']:
   with self.assertRaises(probe.ProbeError):probe.parse_proc_stat(value)
 def test_identity_change(self):
  before=probe.read_identity(os.getpid());after=dict(before);after['start_ticks']+=1
  with self.assertRaises(probe.ProbeError):probe.assert_same_identity('self',before,after)
 def test_command_error(self):
  with patch.object(probe.subprocess,'run',return_value=subprocess.CompletedProcess(['x'],1,'','denied')):
   r=probe.run_command(['x']);self.assertEqual(r['status'],'ERROR');self.assertEqual(r['exit_code'],1)
 def test_command_missing(self):
  with patch.object(probe.subprocess,'run',side_effect=FileNotFoundError('missing')):
   r=probe.run_command(['x']);self.assertEqual(r['status'],'ERROR');self.assertEqual(r['exit_code'],127)
 def test_rdma_missing(self):
  with patch.object(probe.shutil,'which',return_value=None):self.assertEqual(probe.collect_rdma()['status'],'UNAVAILABLE')
 def test_command_timeout(self):
  with patch.object(probe.subprocess,'run',side_effect=subprocess.TimeoutExpired(['x'],5)):
   self.assertEqual(probe.run_command(['x'])['status'],'TIMEOUT')
 def test_rdma_bad_json(self):
  with patch.object(probe,'run_command',return_value={'status':'OBSERVED','stdout':'broken','exit':0}):
   r=probe.collect_rdma();self.assertEqual(r['status'],'PARTIAL')
 def test_no_target_not_pass(self):
  self.assertEqual(probe.collect_report([],[])['status'],'NO_PROCESS_TARGETS')
 def test_platform_rejection(self):
  with patch.object(probe.platform,'system',return_value='Darwin'):
   with self.assertRaises(probe.ProbeError):probe.require_linux_aarch64_root()
 def test_nonroot_rejection(self):
  with patch.object(probe.os,'geteuid',return_value=501):
   with self.assertRaises(probe.ProbeError):probe.require_linux_aarch64_root()
if __name__=='__main__':unittest.main()
