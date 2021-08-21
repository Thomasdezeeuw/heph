//! Module with the type part of a HTTP message head.

pub mod header;
pub mod method;
mod status_code;
pub mod version;

#[doc(no_inline)]
pub use header::{Header, HeaderName, Headers};
#[doc(no_inline)]
pub use method::Method;
pub use status_code::StatusCode;
#[doc(no_inline)]
pub use version::Version;
