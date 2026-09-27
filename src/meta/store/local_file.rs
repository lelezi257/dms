//! Local file persistence for opaque Meta Store snapshots.
//!
//! This backend owns only durable bytes plus a monotonically increasing version.
//! The Store above it owns all semantic validation and state transitions.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use afs_error::{Error, ErrorKind, Result};

use super::{MetaFuture, StoreBackend};

const LOCK_FILE: &str = "LOCK";
const SNAPSHOT_FILE: &str = "snapshot";
const SNAPSHOT_TMP_FILE: &str = "snapshot.tmp";
const WAL_FILE: &str = "wal";
const FRAME_MAGIC: &[u8; 4] = b"AFSL";
const FRAME_HEADER_LEN: usize = 28;
const COMPACT_AFTER_FRAMES: usize = 64;

/// Durable local-file backend for versioned Store snapshots.
#[derive(Clone)]
pub struct LocalFileBackend {
    inner: Arc<LocalFileInner>,
}

struct LocalFileInner {
    dir: PathBuf,
    _lock: File,
    op_lock: Mutex<()>,
}

impl std::fmt::Debug for LocalFileBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalFileBackend")
            .field("dir", &self.inner.dir)
            .finish_non_exhaustive()
    }
}

impl LocalFileBackend {
    /// Opens a local Store backend rooted at `path` and takes an exclusive process lock.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dir = path.as_ref().to_path_buf();
        create_dir_durable(&dir)?;

        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join(LOCK_FILE))?;
        lock_exclusive(&lock)?;
        lock.sync_all()?;
        sync_dir(&dir)?;

        let backend = Self {
            inner: Arc::new(LocalFileInner {
                dir,
                _lock: lock,
                op_lock: Mutex::new(()),
            }),
        };
        let _ = backend.load()?;
        Ok(backend)
    }

    /// Loads the latest complete snapshot, if one has been committed.
    pub fn load(&self) -> Result<Option<(u64, Vec<u8>)>> {
        let _guard = self
            .inner
            .op_lock
            .lock()
            .map_err(|_| internal_error("local file backend lock poisoned"))?;
        self.load_inner()
    }

    /// Appends and fsyncs a new snapshot if `expected_version` is current.
    pub fn commit(&self, expected_version: u64, bytes: &[u8]) -> Result<u64> {
        let _guard = self
            .inner
            .op_lock
            .lock()
            .map_err(|_| internal_error("local file backend lock poisoned"))?;
        let current = self.load_inner()?.map(|(version, _)| version).unwrap_or(0);
        if current != expected_version {
            return Err(failed_precondition(format!(
                "local file backend expected version {expected_version}, found {current}"
            )));
        }

        let new_version = expected_version
            .checked_add(1)
            .ok_or_else(|| failed_precondition("local file backend version overflow"))?;
        let wal_path = self.inner.dir.join(WAL_FILE);
        let wal_existed = wal_path.exists();
        let mut wal = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&wal_path)?;
        if !wal_existed {
            sync_dir(&self.inner.dir)?;
        }
        write_frame(&mut wal, new_version, bytes)?;
        wal.sync_all()?;

        let snapshot_version = read_snapshot(&self.inner.dir.join(SNAPSHOT_FILE))?
            .map(|(version, _)| version)
            .unwrap_or(0);
        let wal_frames = replay_wal(&wal_path, snapshot_version, None)?.frames_after_snapshot;
        if wal_frames >= COMPACT_AFTER_FRAMES {
            self.compact(new_version, bytes)?;
        }
        Ok(new_version)
    }

    fn load_inner(&self) -> Result<Option<(u64, Vec<u8>)>> {
        let mut state = read_snapshot(&self.inner.dir.join(SNAPSHOT_FILE))?;
        let base_version = state.as_ref().map(|(version, _)| *version).unwrap_or(0);
        let replay = replay_wal(&self.inner.dir.join(WAL_FILE), base_version, state.take())?;
        if let Some(truncate_at) = replay.truncate_at {
            truncate_wal(&self.inner.dir.join(WAL_FILE), truncate_at)?;
        }
        Ok(replay.state)
    }

    fn compact(&self, version: u64, bytes: &[u8]) -> Result<()> {
        let tmp_path = self.inner.dir.join(SNAPSHOT_TMP_FILE);
        let snapshot_path = self.inner.dir.join(SNAPSHOT_FILE);
        {
            let mut tmp = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp_path)?;
            write_frame(&mut tmp, version, bytes)?;
            tmp.sync_all()?;
        }
        fs::rename(&tmp_path, &snapshot_path)?;
        sync_dir(&self.inner.dir)?;

        let wal = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.inner.dir.join(WAL_FILE))?;
        wal.sync_all()?;
        sync_dir(&self.inner.dir)?;
        Ok(())
    }
}

impl StoreBackend for LocalFileBackend {
    fn load(&self) -> MetaFuture<'_, Option<(u64, Vec<u8>)>> {
        let backend = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || LocalFileBackend::load(&backend))
                .await
                .map_err(|error| {
                    Error::coded(
                        afs_error::IO_UNAVAILABLE,
                        format!("local store load task failed: {error}"),
                    )
                })?
        })
    }

    fn commit(&self, expected_version: u64, bytes: Vec<u8>) -> MetaFuture<'_, u64> {
        let backend = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                LocalFileBackend::commit(&backend, expected_version, &bytes)
            })
            .await
            .map_err(|error| {
                Error::coded(
                    afs_error::IO_UNAVAILABLE,
                    format!("local store commit task failed: {error}"),
                )
            })?
        })
    }
}

impl Drop for LocalFileInner {
    fn drop(&mut self) {
        let _ = unlock(&self._lock);
    }
}

struct ReplayResult {
    state: Option<(u64, Vec<u8>)>,
    frames_after_snapshot: usize,
    truncate_at: Option<u64>,
}

fn read_snapshot(path: &Path) -> Result<Option<(u64, Vec<u8>)>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Err(data_loss("snapshot file is empty"));
    }
    let (frame, end) = decode_frame(&bytes, 0, TailPolicy::Reject).map_err(frame_error_to_error)?;
    if end != bytes.len() {
        return Err(data_loss("snapshot has trailing bytes"));
    }
    Ok(Some((frame.version, frame.payload)))
}

fn replay_wal(
    path: &Path,
    snapshot_version: u64,
    snapshot: Option<(u64, Vec<u8>)>,
) -> Result<ReplayResult> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReplayResult {
                state: snapshot,
                frames_after_snapshot: 0,
                truncate_at: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let mut offset = 0usize;
    let mut truncate_at = None;
    let mut state = snapshot;
    let mut current_version = snapshot_version;
    let mut frames_after_snapshot = 0usize;

    while offset < bytes.len() {
        match decode_frame(&bytes, offset, TailPolicy::Truncate) {
            Ok((frame, next)) => {
                if frame.version > current_version {
                    let expected = current_version.checked_add(1).ok_or_else(|| {
                        failed_precondition("local file backend version overflow")
                    })?;
                    if frame.version != expected {
                        return Err(data_loss(format!(
                            "wal version gap: expected {expected}, found {}",
                            frame.version
                        )));
                    }
                    current_version = frame.version;
                    state = Some((frame.version, frame.payload));
                    frames_after_snapshot += 1;
                }
                offset = next;
            }
            Err(FrameError::Incomplete) => {
                truncate_at = Some(u64::try_from(offset).map_err(|_| data_loss("wal too large"))?);
                break;
            }
            Err(FrameError::Corrupt(message)) => return Err(data_loss(message)),
        }
    }

    Ok(ReplayResult {
        state,
        frames_after_snapshot,
        truncate_at,
    })
}

fn truncate_wal(path: &Path, len: u64) -> Result<()> {
    let wal = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    wal.set_len(len)?;
    wal.sync_all()?;
    Ok(())
}

struct Frame {
    version: u64,
    payload: Vec<u8>,
}

enum TailPolicy {
    Truncate,
    Reject,
}

enum FrameError {
    Incomplete,
    Corrupt(String),
}

fn write_frame(file: &mut File, version: u64, payload: &[u8]) -> Result<()> {
    let len = u64::try_from(payload.len())
        .map_err(|_| failed_precondition("local file backend payload too large"))?;
    let checksum = checksum(version, len, payload);
    file.write_all(FRAME_MAGIC)?;
    file.write_all(&version.to_le_bytes())?;
    file.write_all(&len.to_le_bytes())?;
    file.write_all(&checksum.to_le_bytes())?;
    file.write_all(payload)?;
    Ok(())
}

fn decode_frame(
    bytes: &[u8],
    offset: usize,
    tail_policy: TailPolicy,
) -> std::result::Result<(Frame, usize), FrameError> {
    if bytes.len() - offset < FRAME_HEADER_LEN {
        return match tail_policy {
            TailPolicy::Truncate => Err(FrameError::Incomplete),
            TailPolicy::Reject => Err(FrameError::Corrupt("incomplete frame header".to_owned())),
        };
    }

    let header = &bytes[offset..offset + FRAME_HEADER_LEN];
    if &header[0..4] != FRAME_MAGIC {
        return Err(FrameError::Corrupt("frame magic mismatch".to_owned()));
    }
    let version = u64::from_le_bytes(header[4..12].try_into().expect("version slice"));
    let len = u64::from_le_bytes(header[12..20].try_into().expect("len slice"));
    let expected_checksum = u64::from_le_bytes(header[20..28].try_into().expect("checksum slice"));
    let payload_len = usize::try_from(len)
        .map_err(|_| FrameError::Corrupt("frame payload length overflows usize".to_owned()))?;
    let payload_start = offset + FRAME_HEADER_LEN;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or_else(|| FrameError::Corrupt("frame payload length overflows offset".to_owned()))?;
    if payload_end > bytes.len() {
        return match tail_policy {
            TailPolicy::Truncate => Err(FrameError::Incomplete),
            TailPolicy::Reject => Err(FrameError::Corrupt("incomplete frame payload".to_owned())),
        };
    }

    let payload = &bytes[payload_start..payload_end];
    let actual_checksum = checksum(version, len, payload);
    if actual_checksum != expected_checksum {
        return Err(FrameError::Corrupt(format!(
            "frame checksum mismatch at offset {offset}"
        )));
    }
    Ok((
        Frame {
            version,
            payload: payload.to_vec(),
        },
        payload_end,
    ))
}

fn checksum(version: u64, len: u64, payload: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in FRAME_MAGIC
        .iter()
        .copied()
        .chain(version.to_le_bytes())
        .chain(len.to_le_bytes())
        .chain(payload.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn create_dir_durable(dir: &Path) -> Result<()> {
    if let Some(parent) = dir.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
        sync_dir(parent)?;
    }
    fs::create_dir_all(dir)?;
    sync_dir(dir)?;
    if let Some(parent) = dir.parent()
        && !parent.as_os_str().is_empty()
    {
        sync_dir(parent)?;
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    match File::open(path) {
        Ok(dir) => dir.sync_all().map_err(Into::into),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn lock_exclusive(file: &File) -> Result<()> {
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => Error::new(
            afs_error::IO_UNAVAILABLE,
            ErrorKind::Unavailable,
            "local file backend is already locked",
        ),
        TryLockError::Error(error) => error.into(),
    })
}

fn unlock(file: &File) -> io::Result<()> {
    file.unlock()
}

fn data_loss(message: impl Into<String>) -> Error {
    Error::new(afs_error::IO_INVALID, ErrorKind::DataLoss, message)
}

fn frame_error_to_error(error: FrameError) -> Error {
    match error {
        FrameError::Incomplete => data_loss("incomplete frame"),
        FrameError::Corrupt(message) => data_loss(message),
    }
}

fn failed_precondition(message: impl Into<String>) -> Error {
    Error::new(
        afs_error::IO_INVALID,
        ErrorKind::FailedPrecondition,
        message,
    )
}

fn internal_error(message: impl Into<String>) -> Error {
    Error::new(afs_error::IO_OTHER, ErrorKind::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};

    #[test]
    fn replays_committed_frames_after_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let backend = LocalFileBackend::open(dir.path()).expect("open backend");
            assert_eq!(backend.load().expect("initial load"), None);
            assert_eq!(backend.commit(0, b"one").expect("commit one"), 1);
            assert_eq!(backend.commit(1, b"two").expect("commit two"), 2);
        }

        let backend = LocalFileBackend::open(dir.path()).expect("reopen backend");
        assert_eq!(
            backend.load().expect("load after reopen"),
            Some((2, b"two".to_vec()))
        );
    }

    #[test]
    fn torn_wal_tail_is_truncated_and_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let backend = LocalFileBackend::open(dir.path()).expect("open backend");
            backend.commit(0, b"stable").expect("commit stable");
        }
        {
            let mut wal = OpenOptions::new()
                .append(true)
                .open(dir.path().join(WAL_FILE))
                .expect("open wal");
            wal.write_all(FRAME_MAGIC).expect("write torn tail");
            wal.sync_all().expect("sync torn tail");
        }

        let backend = LocalFileBackend::open(dir.path()).expect("reopen backend");
        assert_eq!(
            backend.load().expect("load with torn tail"),
            Some((1, b"stable".to_vec()))
        );
        let wal_len = fs::metadata(dir.path().join(WAL_FILE))
            .expect("wal metadata")
            .len();
        assert_eq!(
            wal_len,
            u64::try_from(FRAME_HEADER_LEN + b"stable".len()).unwrap()
        );
    }

    #[test]
    fn checksum_corruption_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let backend = LocalFileBackend::open(dir.path()).expect("open backend");
            backend.commit(0, b"stable").expect("commit stable");
        }
        {
            let mut wal = OpenOptions::new()
                .read(true)
                .write(true)
                .open(dir.path().join(WAL_FILE))
                .expect("open wal");
            wal.seek(SeekFrom::End(-1)).expect("seek last byte");
            wal.write_all(b"x").expect("corrupt last byte");
            wal.sync_all().expect("sync corruption");
        }

        let error = LocalFileBackend::open(dir.path()).expect_err("corruption rejected");
        assert_eq!(error.kind(), ErrorKind::DataLoss);
    }

    #[test]
    fn empty_snapshot_is_corruption_not_fresh_authority() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(SNAPSHOT_FILE), []).expect("write empty snapshot");
        let error = LocalFileBackend::open(dir.path()).expect_err("empty snapshot rejected");
        assert_eq!(error.kind(), ErrorKind::DataLoss);
    }

    #[test]
    fn process_lock_rejects_second_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _backend = LocalFileBackend::open(dir.path()).expect("first open");
        let error = LocalFileBackend::open(dir.path()).expect_err("second open fails");
        assert_eq!(error.kind(), ErrorKind::Unavailable);
    }

    #[test]
    fn compaction_bounds_wal_growth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = LocalFileBackend::open(dir.path()).expect("open backend");
        let mut version = 0;
        for index in 0..70 {
            version = backend
                .commit(version, format!("payload-{index}").as_bytes())
                .expect("commit");
        }
        assert_eq!(
            backend.load().expect("load compacted"),
            Some((70, b"payload-69".to_vec()))
        );
        let wal_len = fs::metadata(dir.path().join(WAL_FILE))
            .expect("wal metadata")
            .len();
        assert!(wal_len < 6 * u64::try_from(FRAME_HEADER_LEN + 16).unwrap());
    }
}
