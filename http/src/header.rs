//! Module with HTTP header related types.

// TODO: impl for `Headers`.
// * FromIterator
// * Extend

use std::borrow::Cow;
use std::convert::AsRef;
use std::fmt;
use std::iter::FusedIterator;

use crate::{cmp_lower_case, is_lower_case, FromBytes};

/// List of headers.
pub struct Headers {
    /// All values appended in a single allocation.
    values: Vec<u8>,
    /// All parts of the headers.
    parts: Vec<HeaderPart>,
}

struct HeaderPart {
    name: HeaderName<'static>,
    /// Indices into `Headers.data`.
    start: usize,
    end: usize,
}

impl Headers {
    /// Empty list of headers.
    pub const EMPTY: Headers = Headers {
        values: Vec::new(),
        parts: Vec::new(),
    };

    /// Creates new `Headers` from `headers`.
    ///
    /// Calls `F` for each header.
    pub(crate) fn from_httparse_headers<F, E>(
        raw_headers: &[httparse::Header<'_>],
        mut f: F,
    ) -> Result<Headers, E>
    where
        F: FnMut(&HeaderName, &[u8]) -> Result<(), E>,
    {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            let name = HeaderName::from_str(header.name);
            let value = header.value;
            if let Err(err) = f(&name, value) {
                return Err(err);
            }
            headers._add(name, value);
        }
        Ok(headers)
    }

    /// Returns the number of headers.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Add a new `header`.
    ///
    /// # Notes
    ///
    /// This doesn't check for duplicate headers, it just adds it to the list of
    /// headers.
    pub fn add<'v>(&mut self, header: Header<'static, 'v>) {
        self._add(header.name, header.value)
    }

    fn _add(&mut self, name: HeaderName<'static>, value: &[u8]) {
        let start = self.values.len();
        self.values.extend_from_slice(value);
        let end = self.values.len();
        self.parts.push(HeaderPart { name, start, end });
    }

    /// Get the header with `name`, if any.
    ///
    /// # Notes
    ///
    /// If all you need is the header value you can use [`Headers::get_value`].
    pub fn get<'a>(&'a self, name: &HeaderName<'_>) -> Option<Header<'a, 'a>> {
        for part in self.parts.iter() {
            if part.name == *name {
                return Some(Header {
                    name: part.name.borrow(),
                    value: &self.values[part.start..part.end],
                });
            }
        }
        None
    }

    /// Get the header's value with `name`, if any.
    pub fn get_value<'a>(&'a self, name: &HeaderName) -> Option<&'a [u8]> {
        for part in self.parts.iter() {
            if part.name == *name {
                return Some(&self.values[part.start..part.end]);
            }
        }
        None
    }

    // TODO: remove header?

    /// Returns an iterator over all headers.
    ///
    /// The order is unspecified.
    pub fn iter<'a>(&'a self) -> Iter<'a> {
        Iter {
            headers: self,
            pos: 0,
        }
    }
}

impl From<Header<'static, '_>> for Headers {
    fn from(header: Header<'static, '_>) -> Headers {
        Headers {
            values: header.value.to_vec(),
            parts: vec![HeaderPart {
                name: header.name,
                start: 0,
                end: header.value.len(),
            }],
        }
    }
}

/*
/// # Notes
///
/// This clones the [`HeaderName`] in each header. For static headers, i.e. the
/// `HeaderName::*` constants, this is a cheap operation, for customer headers
/// this requires an allocation.
impl From<&'_ [Header<'_>]> for Headers {
    fn from(raw_headers: &'_ [Header<'_>]) -> Headers {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            headers._add(header.name.clone(), header.value);
        }
        headers
    }
}
*/

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_map();
        for part in self.parts.iter() {
            let value = &self.values[part.start..part.end];
            if let Ok(str) = std::str::from_utf8(value) {
                f.entry(&part.name, &str);
            } else {
                f.entry(&part.name, &value);
            }
        }
        f.finish()
    }
}

/// Iterator for [`Headers`], see [`Headers::iter`].
pub struct Iter<'a> {
    headers: &'a Headers,
    pos: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Header<'a, 'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.headers.parts.get(self.pos).map(|part| {
            let header = Header {
                name: part.name.borrow(),
                value: &self.headers.values[part.start..part.end],
            };
            self.pos += 1;
            header
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }

    fn count(self) -> usize {
        self.len()
    }
}

impl<'a> ExactSizeIterator for Iter<'a> {
    fn len(&self) -> usize {
        self.headers.len() - self.pos
    }
}

impl<'a> FusedIterator for Iter<'a> {}

/// HTTP header.
///
/// RFC 7230 section 3.2.
#[derive(Clone)]
pub struct Header<'n, 'v> {
    name: HeaderName<'n>,
    value: &'v [u8],
}

impl<'n, 'v> Header<'n, 'v> {
    /// Create a new `Header`.
    ///
    /// # Notes
    ///
    /// `value` MUST NOT contain `\r\n`.
    pub const fn new(name: HeaderName<'n>, value: &'v [u8]) -> Header<'n, 'v> {
        debug_assert!(no_crlf(value), "header value contains CRLF ('\\r\\n')");
        Header { name, value }
    }

    /// Returns the name of the header.
    pub const fn name(&self) -> &HeaderName<'n> {
        &self.name
    }

    /// Returns the value of the header.
    pub const fn value(&self) -> &[u8] {
        self.value
    }

    /// Parse the value of the header using `T`'s [`FromBytes`] implementation.
    pub fn parse<T>(&self) -> Result<T, T::Err>
    where
        T: FromBytes<'v>,
    {
        FromBytes::from_bytes(self.value)
    }
}

/// Returns `true` if `value` does not contain `\r\n`.
const fn no_crlf(value: &[u8]) -> bool {
    let mut i = 1;
    while i < value.len() {
        if value[i - 1] == b'\r' && value[i] == b'\n' {
            return false;
        }
        i += 1;
    }
    true
}

impl<'n, 'v> fmt::Debug for Header<'n, 'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("Header");
        f.field("name", &self.name);
        if let Ok(str) = std::str::from_utf8(self.value) {
            f.field("value", &str);
        } else {
            f.field("value", &self.value);
        }
        f.finish()
    }
}

/// HTTP header name.
#[derive(Clone, PartialEq, Eq)]
pub struct HeaderName<'a> {
    /// The value MUST be lower case.
    inner: Cow<'a, str>,
}

/// Macro to create [`Name`] constants.
macro_rules! known_headers {
    ($(
        $length: tt: [
            $( ( $const_name: ident, $http_name: expr ) $(,)* ),+
        ],
    )+) => {
        $($(
            pub const $const_name: HeaderName<'static> = HeaderName::from_lowercase($http_name);
        )+)+

        /// Create a new HTTP header `HeaderName`.
        ///
        /// # Notes
        ///
        /// If `name` is static prefer to use [`HeaderName::from_lowercase`].
        pub fn from_str(name: &str) -> HeaderName<'static> {
            // This first matches on the length of the `name`, then does a
            // case-insensitive compare of the name with all known headers with
            // the same length, returning a static version if a match is found.
            match name.len() {
                $(
                $length => {
                    $(
                    if cmp_lower_case($http_name, name) {
                        return HeaderName::$const_name;
                    }
                    )+
                }
                )+
                _ => {}
            }
            // If it's not a known header return a custom (heap-allocated)
            // header name.
            HeaderName::from(name.to_string())
        }
    }
}

impl HeaderName<'static> {
    // NOTE: we adding here also add to the
    // `functional::header::from_str_known_headers` test.
    known_headers!(
        4: [ (DATE, "date") ],
        5: [ (ALLOW, "allow") ],
        10: [ (USER_AGENT, "user-agent") ],
        12: [ (X_REQUEST_ID, "x-request-id") ],
        14: [ (CONTENT_LENGTH, "content-length") ],
        17: [ (TRANSFER_ENCODING, "transfer-encoding") ],
    );

    /// Create a new HTTP header `HeaderName`.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not all ASCII lowercase.
    pub const fn from_lowercase(name: &'static str) -> HeaderName<'static> {
        assert!(is_lower_case(name), "header name not lowercase");
        HeaderName {
            inner: Cow::Borrowed(name),
        }
    }

    /// Borrow the header name for a shorter lifetime.
    ///
    /// This is used in things like [`Headers::get`] and [`Iter`] for `Headers`
    /// to avoid clone heap-allocated `HeaderName`s.
    fn borrow<'b>(&'b self) -> HeaderName<'b> {
        HeaderName {
            inner: Cow::Borrowed(self.as_ref()),
        }
    }

    /// Returns `true` if `self` is heap allocated.
    ///
    /// # Notes
    ///
    /// This is only header to test [`HeaderName::from_str`], not part of the
    /// stable API.
    #[doc(hidden)]
    pub fn is_heap_allocated(&self) -> bool {
        matches!(self.inner, Cow::Owned(_))
    }
}

impl From<String> for HeaderName<'static> {
    fn from(mut name: String) -> HeaderName<'static> {
        name.make_ascii_lowercase();
        HeaderName {
            inner: Cow::Owned(name),
        }
    }
}

impl<'a> AsRef<str> for HeaderName<'a> {
    fn as_ref(&self) -> &str {
        self.inner.as_ref()
    }
}

impl<'a> PartialEq<str> for HeaderName<'a> {
    fn eq(&self, other: &str) -> bool {
        // NOTE: `self` is always lowercase, per the comment on the `inner`
        // field.
        cmp_lower_case(self.inner.as_ref(), other)
    }
}

impl<'a> PartialEq<&'_ str> for HeaderName<'a> {
    fn eq(&self, other: &&str) -> bool {
        self.eq(*other)
    }
}

impl<'a> fmt::Debug for HeaderName<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl<'a> fmt::Display for HeaderName<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}
