//! Version related types.

use std::fmt;
use std::str::FromStr;

/// HTTP version.
///
/// RFC 9110 section 2.5.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Version {
    /// HTTP/1.0.
    ///
    /// RFC 1945.
    Http10,
    /// HTTP/1.1.
    ///
    /// RFC 9112.
    Http11,
}

impl Version {
    /// Returns the major version.
    pub const fn major(self) -> u8 {
        match self {
            Version::Http10 | Version::Http11 => 1,
        }
    }

    /// Returns the minor version.
    pub const fn minor(self) -> u8 {
        match self {
            Version::Http10 => 0,
            Version::Http11 => 1,
        }
    }

    /// Returns the highest minor version with the same major version as `self`.
    #[must_use]
    pub const fn highest_minor(self) -> Version {
        match self {
            Version::Http10 | Version::Http11 => Version::Http11,
        }
    }

    /// Returns the version as string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by the [`FromStr`] implementation for [`Version`].
#[derive(Copy, Clone, Debug)]
pub struct UnknownVersion;

impl fmt::Display for UnknownVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown HTTP version")
    }
}

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
