use std::{fmt, str};

/// Analogous trait to the [`FromStr`] trait.
///
/// [`FromStr`]: std::str::FromStr
pub trait FromBytes<'a>: Sized {
    type Err;

    fn from_bytes(value: &'a [u8]) -> Result<Self, Self::Err>;
}

#[derive(Debug)]
pub struct ParseIntError;

impl fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid integer")
    }
}

impl FromBytes<'_> for usize {
    type Err = ParseIntError;

    fn from_bytes(src: &[u8]) -> Result<Self, Self::Err> {
        if src.is_empty() {
            return Err(ParseIntError);
        }

        let mut value = 0;
        for b in src.iter().copied() {
            if b >= b'0' && b <= b'9' {
                // TODO: check if this doesn't get compiled away.
                if value >= (usize::MAX / 10) {
                    // Overflow.
                    return Err(ParseIntError);
                }
                value = (value * 10) + (b - b'0') as usize;
            } else {
                return Err(ParseIntError);
            }
        }
        Ok(value)
    }
}

impl<'a> FromBytes<'a> for &'a str {
    type Err = str::Utf8Error;

    fn from_bytes(src: &'a [u8]) -> Result<Self, Self::Err> {
        str::from_utf8(src)
    }
}
