import ast,importlib.util,pathlib
p=pathlib.Path('/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/experiments/afs-acceptance/integration-v51.py')
spec=importlib.util.spec_from_file_location('integration_v51',p); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
original_guest=m.guest
def resume_guest(k,code,timeout=120):
 if k=='a' and '; r.mkdir()' in code:
  code=code.replace('; r.mkdir()',"\nassert r.is_dir() and not list((r/'run').iterdir()) and not list((r/'prefix/bin').iterdir()),r")
 return original_guest(k,code,timeout)
m.guest=resume_guest
fn=next(n for n in ast.parse(p.read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='prepare')
# Skip only OUT.mkdir and already captured both-VM preflight. Preserve embedded
# guest literals exactly, and finish the same unlaunched runtime preparation.
module=ast.Module(body=fn.body[2:],type_ignores=[])
exec(compile(ast.fix_missing_locations(module),str(p)+'::resume','exec'),m.__dict__)
