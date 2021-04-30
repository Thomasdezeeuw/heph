use std::fmt;
use std::str::FromStr;

/// HTTP version.
///
/// RFC 7231 section 2.6.
#[derive(Copy, Clone, Debug)]
pub enum Version {
    /// HTTP/1.0.
    Http10,
    /// HTTP/1.1.
    Http11,
}

impl Version {
    /// Returns the highest minor version with the same major version as `self`.
    ///
    /// According to RFC 7230 section 2.6:
    /// > A server SHOULD send a response version equal to the highest version
    /// > to which the server is conformant that has a major version less than or
    /// > equal to the one received in the request.
    ///
    /// This function can be used to return the highest version given a major
    /// version.
    pub const fn highest_minor(self) -> Version {
        match self {
            Version::Http10 | Version::Http11 => Version::Http11,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        })
    }
}

/// Error returned by the [`FromStr`] implementation for [`Version`].
#[derive(Copy, Clone, Debug)]
pub struct UnknownVersion;

impl FromStr for Version {
    type Err = UnknownVersion;

    fn from_str(method: &str) -> Result<Self, Self::Err> {
        match method {
            "HTTP/1.0" => Ok(Version::Http10),
            "HTTP/1.1" => Ok(Version::Http11),
            _ => Err(UnknownVersion),
        }
    }
}
