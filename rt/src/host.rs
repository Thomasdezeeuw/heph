//! Host machine information.

use std::ffi::CStr;
use std::{fmt, io, mem};

use crate::syscall;

/// Returns (OS name and version, hostname).
///
/// Uses `uname(2)`.
pub(crate) fn info() -> io::Result<(Box<str>, Box<str>)> {
    // We could also use `std::env::consts::OS`, but this looks better.
    const OS: &str = cfg_select! {
        target_os = "linux" => "GNU/Linux",
        target_os = "freebsd" => "FreeBSD",
        target_os = "macos" => "macOS",
    };

    let mut uname_info: libc::utsname = unsafe { mem::zeroed() };
    _ = syscall!(uname(&raw mut uname_info)).map_err(|err| {
        io::Error::new(err.kind(), format!("failed to get OS information: {err}"))
    })?;

    // SAFETY: call to `uname(2)` above ensures `uname_info` is initialised.
    let sysname = unsafe { CStr::from_ptr(uname_info.sysname.as_ptr().cast()).to_string_lossy() };
    let release = unsafe { CStr::from_ptr(uname_info.release.as_ptr().cast()).to_string_lossy() };
    let version = unsafe { CStr::from_ptr(uname_info.version.as_ptr().cast()).to_string_lossy() };
    let nodename = unsafe { CStr::from_ptr(uname_info.nodename.as_ptr().cast()).to_string_lossy() };

    let os = format!("{OS} ({sysname} {release} {version})").into_boxed_str();
    let hostname = nodename.into_owned().into_boxed_str();
    Ok((os, hostname))
}

/// Universally Unique IDentifier (UUID), see [RFC 4122].
///
/// [RFC 4122]: https://datatracker.ietf.org/doc/html/rfc4122
#[derive(Copy, Clone)]
#[allow(clippy::doc_markdown)]
pub(crate) struct Uuid(u128);

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always force a length of 32.
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Get the host id by reading `/etc/machine-id` on Linux or `/etc/hostid` on
/// FreeBSD.
#[cfg(any(target_os = "freebsd", target_os = "linux"))]
pub(crate) fn id() -> io::Result<Uuid> {
    use std::fs::File;
    use std::io::Read;

    // For Linux: <https://www.freedesktop.org/software/systemd/man/machine-id.html>.
    // For FreeBSD there are no docs, but a bug tracker:
    // <https://bugs.freebsd.org/bugzilla/show_bug.cgi?id=255293>.
    const PATH: &str = cfg_select! {
        target_os = "linux" => "/etc/machine-id",
        target_os = "freebsd" => "/etc/hostid",
    };
    const EXPECTED_SIZE: usize = cfg_select! {
        target_os = "linux" => 32,
        target_os = "freebsd" => 36,
    };

    let mut buf = [0; EXPECTED_SIZE];
    let mut file = File::open(PATH)?;
    let n = file.read(&mut buf).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to get host id: can't open '{PATH}': {err}"),
        )
    })?;

    if n == EXPECTED_SIZE {
        #[cfg(target_os = "linux")]
        let res = from_hex(&buf[..EXPECTED_SIZE]);
        #[cfg(target_os = "freebsd")]
        let res = from_hex_hyphenated(&buf[..EXPECTED_SIZE]);

        res.map_err(|()| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to get host id: invalid '{PATH}' format: input is not hex"),
            )
        })
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "failed to get host id: can't read '{PATH}', invalid format: only read {n} bytes (expected {EXPECTED_SIZE})"
            ),
        ))
    }
}

/// `input` should be 32 bytes long.
#[cfg(target_os = "linux")]
fn from_hex(input: &[u8]) -> Result<Uuid, ()> {
    let mut bytes = [0; 16];
    for (idx, chunk) in input.chunks_exact(2).enumerate() {
        let lower = from_hex_byte(chunk[1])?;
        let higher = from_hex_byte(chunk[0])?;
        bytes[idx] = lower | (higher << 4);
    }
    Ok(Uuid(u128::from_be_bytes(bytes)))
}

/// `input` should be 36 bytes long.
#[cfg(target_os = "freebsd")]
fn from_hex_hyphenated(input: &[u8]) -> Result<Uuid, ()> {
    let mut bytes = [0; 16];
    let mut idx = 0;

    // Groups of 8, 4, 4, 4, 12 bytes.
    let groups: [std::ops::Range<usize>; 5] = [0..8, 9..13, 14..18, 19..23, 24..36];

    for group in groups {
        let group_end = group.end;
        for chunk in input[group].chunks_exact(2) {
            let lower = from_hex_byte(chunk[1])?;
            let higher = from_hex_byte(chunk[0])?;
            bytes[idx] = lower | (higher << 4);
            idx += 1;
        }

        if let Some(b) = input.get(group_end) {
            if *b != b'-' {
                return Err(());
            }
        }
    }

    Ok(Uuid(u128::from_be_bytes(bytes)))
}

#[cfg(any(target_os = "freebsd", target_os = "linux"))]
const fn from_hex_byte(b: u8) -> Result<u8, ()> {
    match b {
        b'A'..=b'F' => Ok(b - b'A' + 10),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'0'..=b'9' => Ok(b - b'0'),
        _ => Err(()),
    }
}

/// Gets the host id by calling `gethostuuid` on macOS.
#[cfg(target_os = "macos")]
pub(crate) fn id() -> io::Result<Uuid> {
    let mut bytes = [0; 16];
    let timeout = libc::timespec {
        tv_sec: 1, // This shouldn't block, but just in case. SQLite does this also.
        tv_nsec: 0,
    };
    if unsafe { libc::gethostuuid(bytes.as_mut_ptr(), &timeout) } == -1 {
        let os_err = io::Error::last_os_error();
        Err(io::Error::new(
            os_err.kind(),
            format!("failed to get host id: {os_err}"),
        ))
    } else {
        Ok(Uuid(u128::from_be_bytes(bytes)))
    }
}
