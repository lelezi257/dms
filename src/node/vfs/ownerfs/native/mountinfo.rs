//! Strict /proc/<pid>/mountinfo decoding; line order is not mount ownership.

use std::{ffi::OsString, io, os::unix::ffi::OsStringExt, path::PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountInfo {
    pub mount_id: u64,
    pub parent_id: u64,
    pub device_major: u32,
    pub device_minor: u32,
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub mount_options: Vec<String>,
    pub optional_fields: Vec<String>,
    pub filesystem: String,
    pub source: OsString,
    pub super_options: Vec<String>,
}

impl MountInfo {
    /// Preserve arbitrary filename bytes and every stacked record. Live callers
    /// select the actual mount using a kernel mount ID, never "last line wins".
    pub fn parse(input: &[u8]) -> io::Result<Vec<Self>> {
        input
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(Self::parse_line)
            .collect()
    }

    fn parse_line(line: &[u8]) -> io::Result<Self> {
        let fields: Vec<_> = line.split(|b| *b == b' ').collect();
        let separator = fields
            .iter()
            .position(|field| *field == b"-")
            .ok_or_else(|| invalid("missing mountinfo separator"))?;
        if separator < 6
            || fields.len() != separator + 4
            || fields.iter().any(|field| field.is_empty())
        {
            return Err(invalid("invalid mountinfo field count"));
        }
        let mount_id = number(fields[0])?;
        if mount_id == 0 {
            return Err(invalid("zero mount ID"));
        }
        let device: Vec<_> = fields[2].split(|b| *b == b':').collect();
        if device.len() != 2 {
            return Err(invalid("invalid device identity"));
        }
        let device_major =
            u32::try_from(number(device[0])?).map_err(|_| invalid("device major overflow"))?;
        let device_minor =
            u32::try_from(number(device[1])?).map_err(|_| invalid("device minor overflow"))?;
        let root = PathBuf::from(decode(fields[3])?);
        let mount_point = PathBuf::from(decode(fields[4])?);
        if !root.is_absolute() || !mount_point.is_absolute() {
            return Err(invalid("relative mountinfo path"));
        }
        Ok(Self {
            mount_id,
            parent_id: number(fields[1])?,
            device_major,
            device_minor,
            root,
            mount_point,
            mount_options: options(fields[5])?,
            optional_fields: fields[6..separator]
                .iter()
                .map(|f| text(f))
                .collect::<io::Result<_>>()?,
            filesystem: text(fields[separator + 1])?,
            source: decode(fields[separator + 2])?,
            super_options: options(fields[separator + 3])?,
        })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn text(value: &[u8]) -> io::Result<String> {
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| invalid("non-UTF8 mountinfo field"))
}
fn options(value: &[u8]) -> io::Result<Vec<String>> {
    if value.split(|b| *b == b',').any(|s| s.is_empty()) {
        return Err(invalid("empty mount option"));
    }
    value.split(|b| *b == b',').map(text).collect()
}
fn number(value: &[u8]) -> io::Result<u64> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(invalid("invalid numeric mountinfo field"));
    }
    text(value)?
        .parse()
        .map_err(|_| invalid("mountinfo integer overflow"))
}
fn decode(value: &[u8]) -> io::Result<OsString> {
    let mut output = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let byte = if value[index] == b'\\' {
            let octal = value
                .get(index + 1..index + 4)
                .ok_or_else(|| invalid("truncated path escape"))?;
            if !octal.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return Err(invalid("invalid path escape"));
            }
            let decoded = u16::from(octal[0] - b'0') * 64
                + u16::from(octal[1] - b'0') * 8
                + u16::from(octal[2] - b'0');
            index += 4;
            u8::try_from(decoded).map_err(|_| invalid("path escape overflow"))?
        } else {
            let byte = value[index];
            index += 1;
            byte
        };
        if byte == 0 {
            return Err(invalid("NUL in mountinfo path"));
        }
        output.push(byte);
    }
    Ok(OsString::from_vec(output))
}
