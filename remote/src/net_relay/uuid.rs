use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use getrandom::getrandom;
use log::warn;
use serde::de::{self, Deserialize, Deserializer, Unexpected, Visitor};
use serde::{Serialize, Serializer};

/// Quick and dirty UUID generator.
///
/// # Notes
///
/// This does not create random UUIDs and following UUIDs can be easily guessed
/// based on the previous UUIDs.
pub(crate) struct UuidGenerator(u64, u64);

impl UuidGenerator {
    /// Create a new generator.
    #[allow(clippy::unreadable_literal)]
    pub(crate) fn new() -> UuidGenerator {
        let mut bytes = [0; 16];
        match getrandom(&mut bytes) {
            Ok(()) => {
                let b0 = u64::from_be_bytes(TryInto::try_into(&bytes[0..8]).unwrap());
                let b1 = u64::from_be_bytes(TryInto::try_into(&bytes[8..16]).unwrap());
                UuidGenerator(b0, b1)
            }
            Err(err) => {
                warn!("unable to get random bytes, using a fallback: {err}");
                UuidGenerator(9396178701149223067, 6169990013871724815)
            }
        }
    }

    /// Generate the next UUID.
    #[allow(clippy::unreadable_literal)]
    pub(crate) fn next(&mut self) -> Uuid {
        let bytes1 = self.0;
        let bytes0 = self.1;
        self.0 = self.0.wrapping_add(self.1);
        self.1 = self.1.wrapping_add(92478483931537517);
        let mut bytes = (u128::from(bytes0) + (u128::from(bytes1) << 64)).to_ne_bytes();

        // Set the variant to RFC4122 (section 4.1.1).
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        // Set the version to 4 (random) (section 4.1.3).
        bytes[6] = (bytes[6] & 0x0f) | (4 << 4);

        Uuid(bytes)
    }
}

/// Universally Unique(-ish) Identifier (UUID).
#[derive(Copy, Clone)]
pub(crate) struct Uuid([u8; 16]);

impl Eq for Uuid {}

impl PartialEq for Uuid {
    fn eq(&self, other: &Self) -> bool {
        u128::from_ne_bytes(self.0) == u128::from_ne_bytes(other.0)
    }
}

impl Hash for Uuid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        u128::from_ne_bytes(self.0).hash(state);
    }
}

/// Groups of 8, 4, 4, 4, 12 bytes.
const HYPHENS: [Range<usize>; 5] = [0..8, 9..13, 14..18, 19..23, 24..36];
/// 16 characters used to represents bytes in hexadecimal.
const HEX_CHARS: [u8; 16] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'a', b'b', b'c', b'd', b'e', b'f',
];
/// Positions of the bytes for each grouping.
const BYTE_POSITIONS: [usize; 6] = [0, 4, 6, 8, 10, 16];
/// Positions of the hypens (`-`).
const HYPHEN_POSITIONS: [usize; 4] = [8, 13, 18, 23];

// NOTE: manually implementing this instead of deriving to not pull in a bunch
// of dependencies.
impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            struct StrVisitor;

            impl<'de> Visitor<'de> for StrVisitor {
                type Value = Uuid;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("UUID string")
                }

                fn visit_str<E>(self, input: &str) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    let res = match input.len() {
                        32 => from_hex(input.as_bytes()),
                        36 => from_hex_hyphenated(input.as_bytes()),
                        n => return Err(de::Error::invalid_length(n, &"32 or 36 bytes")),
                    };
                    match res {
                        Ok(uuid) => Ok(uuid),
                        Err(()) => Err(de::Error::invalid_value(
                            Unexpected::Str(input),
                            &"UUID string",
                        )),
                    }
                }
            }

            deserializer.deserialize_str(StrVisitor)
        } else {
            struct BytesVisitor;

            impl<'de> Visitor<'de> for BytesVisitor {
                type Value = Uuid;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("UUID bytes")
                }

                fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    match TryFrom::try_from(bytes) {
                        Ok(bytes) => Ok(Uuid(bytes)),
                        Err(_) => Err(de::Error::invalid_length(bytes.len(), &"32 bytes")),
                    }
                }
            }

            deserializer.deserialize_str(BytesVisitor)
        }
    }
}

/// `input` must be 32 bytes long.
fn from_hex(input: &[u8]) -> Result<Uuid, ()> {
    _ = input[31];
    let mut bytes = [0; 16];
    for (idx, chunk) in input.chunks_exact(2).enumerate() {
        let lower = from_hex_byte(chunk[1])?;
        let higher = from_hex_byte(chunk[0])?;
        bytes[idx] = lower | (higher << 4);
    }
    Ok(Uuid(bytes))
}

/// `input` must be 36 bytes long.
fn from_hex_hyphenated(input: &[u8]) -> Result<Uuid, ()> {
    _ = input[35];
    let mut bytes = [0; 16];
    let mut idx = 0;
    for group in HYPHENS {
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
    Ok(Uuid(bytes))
}

const fn from_hex_byte(b: u8) -> Result<u8, ()> {
    match b {
        b'A'..=b'F' => Ok(b - b'A' + 10),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'0'..=b'9' => Ok(b - b'0'),
        _ => Err(()),
    }
}

impl Serialize for Uuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let mut buf = [0; 36];
            for group in 0..5 {
                for idx in BYTE_POSITIONS[group]..BYTE_POSITIONS[group + 1] {
                    let byte = self.0[idx];
                    let out_idx = group + 2 * idx;
                    buf[out_idx] = HEX_CHARS[usize::from(byte >> 4)];
                    buf[out_idx + 1] = HEX_CHARS[usize::from(byte & 0b1111)];
                }
                if group != 4 {
                    buf[HYPHEN_POSITIONS[group]] = b'-';
                }
            }
            // SAFETY: All bytes are one of `HEX_CHARS`, which are all valid
            // UTF-8 characters.
            serializer.serialize_str(unsafe { std::str::from_utf8_unchecked(&buf) })
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}
