#![allow(
    unreachable_code,
    unused_variables,
    unused_mut,
    dead_code,
    unused_imports
)] // FIXME: remove.
#![feature(
    const_eval_limit,
    const_panic,
    maybe_uninit_array_assume_init,
    maybe_uninit_slice,
    maybe_uninit_uninit_array,
    maybe_uninit_write_slice
)]

use std::convert::AsRef;
use std::fmt;
use std::str::FromStr;

mod body;
mod from_bytes;
pub mod header;
pub mod method;
mod request;
mod response;
pub mod server;
mod status_code;
pub mod version;

pub use body::Body;
pub use from_bytes::FromBytes;
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
        if !matches!(value[i], b'a'..=b'z' | b'-') {
            return false;
        }
        i += 1;
    }
    true
}
