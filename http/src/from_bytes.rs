use std::time::SystemTime;
use std::{fmt, str};

use httpdate::parse_http_date;

/// Analogous trait to [`FromStr`].
///
/// The main use case for this trait in [`Header::parse`]. Because of this the
/// implementations should expect the `value`s passed to be ASCII/UTF-8, but
/// this not true in all cases.
///
/// [`FromStr`]: std::str::FromStr
/// [`Header::parse`]: crate::Header::parse
pub trait FromBytes<'a>: Sized {
    /// Error returned by parsing the bytes.
    type Err;

    /// Parse the `value`.
    fn from_bytes(value: &'a [u8]) -> Result<Self, Self::Err>;
}

#[derive(Debug)]
pub struct ParseIntError;

impl fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid integer")
    }
}

macro_rules! int_impl {
    ($( $ty: ty ),+) => {
        $(
        impl FromBytes<'_> for $ty {
            type Err = ParseIntError;

            fn from_bytes(src: &[u8]) -> Result<Self, Self::Err> {
                if src.is_empty() {
                    return Err(ParseIntError);
                }

                let mut value: $ty = 0;
                for b in src.iter().copied() {
                    if b >= b'0' && b <= b'9' {
                        match value.checked_mul(10) {
                            Some(v) => value = v,
                            None => return Err(ParseIntError),
                        }
                        match value.checked_add((b - b'0') as $ty) {
                            Some(v) => value = v,
                            None => return Err(ParseIntError),
                        }
                    } else {
                        return Err(ParseIntError);
                    }
                }
                Ok(value)
            }
        }
        )+
    };
}

int_impl!(u8, u16, u32, u64, usize);

impl<'a> FromBytes<'a> for &'a str {
    type Err = str::Utf8Error;

    fn from_bytes(value: &'a [u8]) -> Result<Self, Self::Err> {
        str::from_utf8(value)
    }
}

#[derive(Debug)]
pub struct ParseTimeError;

impl fmt::Display for ParseTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid time")
    }
}

/// Parses the value following RFC7231 section 7.1.1.1.
impl FromBytes<'_> for SystemTime {
    type Err = ParseTimeError;

    fn from_bytes(value: &[u8]) -> Result<Self, Self::Err> {
        match str::from_utf8(value) {
            Ok(value) => match parse_http_date(value) {
                Ok(time) => Ok(time),
                Err(_) => Err(ParseTimeError),
            },
            Err(_) => Err(ParseTimeError),
        }
    }
}
