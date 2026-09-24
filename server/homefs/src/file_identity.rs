use std::{fs::File, io, os::fd::AsRawFd};

const MAX_HANDLE_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub mount_id: i32,
    pub handle_type: i32,
    pub handle: Vec<u8>,
}

impl FileIdentity {
    pub fn from_file(file: &File) -> io::Result<Self> {
        #[repr(C)]
        struct HandleBuffer {
            handle_bytes: u32,
            handle_type: i32,
            handle: [u8; MAX_HANDLE_BYTES],
        }
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        let mut buffer = HandleBuffer {
            handle_bytes: MAX_HANDLE_BYTES as u32,
            handle_type: 0,
            handle: [0; MAX_HANDLE_BYTES],
        };
        let mut mount_id = 0;
        let result = unsafe {
            libc::name_to_handle_at(
                file.as_raw_fd(),
                c"".as_ptr(),
                (&raw mut buffer).cast::<libc::file_handle>(),
                &raw mut mount_id,
                libc::AT_EMPTY_PATH,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if buffer.handle_bytes == 0 || buffer.handle_bytes as usize > MAX_HANDLE_BYTES {
            return Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP));
        }
        Ok(Self {
            dev: stat.st_dev,
            ino: stat.st_ino,
            mount_id,
            handle_type: buffer.handle_type,
            handle: buffer.handle[..buffer.handle_bytes as usize].to_vec(),
        })
    }
}
