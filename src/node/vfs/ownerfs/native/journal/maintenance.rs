//! Bounded, explicit maintenance under the manager-private directory lock.
//! Unknown files and uncommitted/future identities never become recovery state.
use super::{
    JournalSnapshot, MAX_BYTES, MountJournal, OrphanMaintenance, check_regular, errno, openat,
    validate_snapshot,
};
use std::{
    ffi::CString,
    fs::{self, File, Metadata},
    io::{self, Read},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};

const MAX_SCAN: usize = 256;

pub(super) fn cleanup(journal: &mut MountJournal) -> io::Result<OrphanMaintenance> {
    let committed = journal.load()?;
    // Collect the bounded directory inventory before removing anything. The
    // descriptor remains live, so this proc path identifies the pinned dir.
    let entries = fs::read_dir(format!(
        "/proc/thread-self/fd/{}",
        journal.directory.as_raw_fd()
    ))?
    .take(MAX_SCAN + 1)
    .collect::<io::Result<Vec<_>>>()?;
    if entries.len() > MAX_SCAN {
        return Err(errno(libc::ENOSPC));
    }
    let mut report = OrphanMaintenance::default();
    let mut candidates = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        // Lossy spelling is diagnostic only. The strict ASCII temp grammar
        // below excludes every non-UTF-8 name before fd-relative operations.
        let name = name.to_string_lossy().into_owned();
        if name == "state.json" || name == "manager.lock" {
            continue;
        }
        if !temporary_name(&name) {
            report.retained.push(name);
            continue;
        }
        match candidate(journal, &committed, &name) {
            Ok(Some(file)) => candidates.push((name, file)),
            Ok(None) => report.retained.push(name),
            Err(error) if matches!(error.raw_os_error(), Some(libc::EIO | libc::ESTALE)) => {
                return Err(error);
            }
            Err(_) => report.retained.push(name),
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, file) in candidates {
        let expected = file.metadata()?;
        let current = match openat(&journal.directory, &name, libc::O_RDONLY) {
            Ok(file) => file,
            Err(_) => {
                report.retained.push(name);
                continue;
            }
        };
        if check_regular(&current).is_err() || !same_file(&expected, &current.metadata()?) {
            report.retained.push(name);
            continue;
        }
        // Only cooperative journal writers access this private directory;
        // its exclusive writer lock is held throughout. Same-UID privileged
        // interference violates that control-plane boundary, as for store().
        journal.uncertain = true;
        unlinkat(
            &journal.directory,
            &CString::new(name.clone()).map_err(io::Error::other)?,
        )?;
        report.removed.push(name);
    }
    if journal.uncertain {
        journal.directory.sync_all()?;
        journal.uncertain = false;
    }
    report.retained.sort();
    Ok(report)
}

fn candidate(
    journal: &MountJournal,
    committed: &JournalSnapshot,
    name: &str,
) -> io::Result<Option<File>> {
    let mut file = openat(&journal.directory, name, libc::O_RDONLY)?;
    check_regular(&file)?;
    if file.metadata()?.len() > MAX_BYTES as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Ok(None);
    }
    let Ok(snapshot) = serde_json::from_slice::<JournalSnapshot>(&bytes) else {
        return Ok(None);
    };
    if validate_snapshot(&snapshot, journal.capacity).is_err()
        || snapshot.boot_id != committed.boot_id
        || snapshot.namespace != committed.namespace
        || snapshot.records.iter().any(|old| {
            !committed.records.iter().any(|current| {
                current.spec == old.spec
                    && old.status.operation_seq <= current.status.operation_seq
                    && old.retired.iter().all(|id| current.retired.contains(id))
            })
        })
    {
        return Ok(None);
    }
    Ok(Some(file))
}

fn temporary_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".state-") else {
        return false;
    };
    let Some((pid, stamp)) = rest.split_once('-') else {
        return false;
    };
    name.len() <= 80
        && !pid.starts_with('0')
        && !stamp.starts_with('0')
        && pid.bytes().all(|b| b.is_ascii_digit())
        && stamp.bytes().all(|b| b.is_ascii_digit())
        && pid.parse::<u32>().is_ok_and(|value| value > 0)
        && stamp.parse::<u128>().is_ok_and(|value| value > 0)
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
#[allow(unsafe_code)]
fn unlinkat(directory: &File, name: &std::ffi::CStr) -> io::Result<()> {
    // SAFETY: the live directory fd and single NUL-terminated component stay
    // valid for this synchronous syscall. Zero flags cannot remove a directory.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
