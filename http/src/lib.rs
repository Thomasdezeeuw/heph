//! HTTP/1.1 implementation for Heph.

#![feature(
    async_stream,
    const_fn_trait_bound,
    const_mut_refs,
    const_panic,
    generic_associated_types,
    io_slice_advance,
    maybe_uninit_write_slice,
    ready_macro,
    stmt_expr_attributes
)]
#![allow(incomplete_features)] // NOTE: for `generic_associated_types`.
#![warn(
    anonymous_parameters,
    bare_trait_objects,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    variant_size_differences
)]

pub mod body;
pub mod header;
pub mod method;
mod request;
mod response;
pub mod server;
mod status_code;
pub mod version;

#[doc(no_inline)]
pub use body::Body;
#[doc(no_inline)]
pub use header::{Header, HeaderName, Headers};
#[doc(no_inline)]
pub use method::Method;
pub use request::Request;
pub use response::Response;
#[doc(no_inline)]
pub use server::{Connection, HttpServer};
pub use status_code::StatusCode;
#[doc(no_inline)]
pub use version::Version;

/// Maximum size of the HTTP head (the start line and the headers).
///
/// RFC 7230 section 3.1.1 recommends "all HTTP senders and recipients support,
/// at a minimum, request-line lengths of 8000 octets."
pub const MAX_HEAD_SIZE: usize = 16384;

/// Maximum number of headers parsed from a single [`Request`]/[`Response`].
pub const MAX_HEADERS: usize = 64;

/// Minimum amount of bytes read from the connection or the buffer will be
/// grown.
const MIN_READ_SIZE: usize = 4096;

/// Size of the buffer used in [`server::Connection`] and [`Client`].
const BUF_SIZE: usize = 8192;

/// Map a `version` byte to a [`Version`].
const fn map_version_byte(version: u8) -> Version {
    match version {
        0 => Version::Http10,
        // RFC 7230 section 2.6:
        // > A server SHOULD send a response version equal to
        // > the highest version to which the server is
        // > conformant that has a major version less than or
        // > equal to the one received in the request.
        // HTTP/1.1 is the highest we support.
        _ => Version::Http11,
    }
}

/// Trim whitespace from `value`.
fn trim_ws(value: &[u8]) -> &[u8] {
    let len = value.len();
    if len == 0 {
        return value;
    }
    let mut start = 0;
    while start < len {
        if !value[start].is_ascii_whitespace() {
            break;
        }
        start += 1;
    }
    let mut end = len - 1;
    while end > start {
        if !value[end].is_ascii_whitespace() {
            break;
        }
        end -= 1;
    }
    // TODO: make this `const`.
    &value[start..=end]
}

/// Returns `true` if `lower_case` and `right` are a case-insensitive match.
///
/// # Notes
///
/// `lower_case` must be lower case!
const fn cmp_lower_case(lower_case: &str, right: &str) -> bool {
    debug_assert!(is_lower_case(lower_case));

    let left = lower_case.as_bytes();
    let right = right.as_bytes();
    let len = left.len();
    if len != right.len() {
        return false;
    }

    let mut i = 0;
    while i < len {
        if left[i] != right[i].to_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    true
}

/// Returns `true` if `value` is all ASCII lowercase.
const fn is_lower_case(value: &str) -> bool {
    let value = value.as_bytes();
    let mut i = 0;
    while i < value.len() {
        // NOTE: allows `-` because it's used in header names.
        if !matches!(value[i], b'0'..=b'9' | b'a'..=b'z' | b'-') {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{cmp_lower_case, is_lower_case, trim_ws};

    #[test]
    fn test_trim_ws() {
        let tests = &[
            ("", ""),
            ("abc", "abc"),
            ("  abc", "abc"),
            ("  abc  ", "abc"),
            ("  gzip, chunked  ", "gzip, chunked"),
        ];
        for (input, expected) in tests {
            let got = trim_ws(input.as_bytes());
            assert_eq!(got, expected.as_bytes(), "input: {}", input);
        }
    }

    #[test]
    fn test_is_lower_case() {
        let tests = &[
            ("", true),
            ("abc", true),
            ("Abc", false),
            ("aBc", false),
            ("AbC", false),
            ("ABC", false),
        ];
        for (input, expected) in tests {
            let got = is_lower_case(input);
            assert_eq!(got, *expected, "input: {}", input);
        }
    }

    #[test]
    fn test_cmp_lower_case() {
        let tests = &[
            ("", "", true),
            ("abc", "abc", true),
            ("abc", "Abc", true),
            ("abc", "aBc", true),
            ("abc", "abC", true),
            ("abc", "ABC", true),
            ("a", "", false),
            ("", "a", false),
            ("abc", "", false),
            ("abc", "d", false),
            ("abc", "de", false),
            ("abc", "def", false),
        ];
        for (lower_case, right, expected) in tests {
            let got = cmp_lower_case(lower_case, right);
            assert_eq!(got, *expected, "input: '{}', '{}'", lower_case, right);
        }
    }
}
