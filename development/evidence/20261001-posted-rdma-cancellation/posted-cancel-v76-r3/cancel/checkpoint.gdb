set pagination off
set confirm off
set print thread-events off
set debuginfod enabled off
set non-stop on
set environment AFS_TEST_RDMA_DEVICE rxe0
set environment AFS_TEST_RDMA_CHECKPOINT /home/lzc.guest/afs-build/evidence/posted-cancel-v76-r3/cancel
python
import gdb, json, pathlib, time
directory = pathlib.Path('/home/lzc.guest/afs-build/evidence/posted-cancel-v76-r3/cancel')
def record(value):
    with (directory / 'debugger.jsonl').open('a') as stream:
        stream.write(json.dumps(value) + '\n')
def exited(event):
    record({'event': 'exit', 'code': getattr(event, 'exit_code', None)})
gdb.events.exited.connect(exited)
class Posted(gdb.Breakpoint):
    def stop(self):
        record({'event': 'posted', 'pid': gdb.selected_inferior().pid,
                'thread': list(gdb.selected_thread().ptid),
                'gdb_thread': gdb.selected_thread().global_num,
                'operation': int(gdb.parse_and_eval('operation')),
                'bytes': int(gdb.parse_and_eval('len')),
                'data_wr_id': int(gdb.parse_and_eval('wr.wr_id')),
                'poisoned': bool(gdb.parse_and_eval('e->poisoned')),
                'qp': int(gdb.parse_and_eval('e->qp->qp_num')),
                'monotonic': time.monotonic()})
        (directory / 'posted').write_text('WQE accepted; CQ unconsumed')
        # Return promptly: GDB must process clone/vfork/exit events
        # for the other running threads while this worker is stopped.
        return True
checkpoint = Posted('native/rdma.c:339')
checkpoint.condition = 'operation == 1 && len == 4096'
end
run --exact posted_rdma_cancel_keeps_endpoint_until_worker_drains --ignored --nocapture --test-threads=1
