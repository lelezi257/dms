# Portable checker path correction

The initial ctl checker invocation used the host absolute workspace path.
That path is not mounted in ctl, so Python reports file not found (Errno2).
The surrounding host multi-command invocation ended0 after copying an empty
output; that is not a validation PASS. Its empty report is replaced only by
the later real checker output. The checker bundle is transferred by tar to
ctl ext4 and rerun there; product source/Rust gates are unchanged.
