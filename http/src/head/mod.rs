//! Module with the type part of a HTTP message head.

use std::fmt;

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

use crate::Request;

/// Head of a [`Request`].
pub struct RequestHead {
    method: Method,
    path: String,
    version: Version,
    headers: Headers,
}

impl RequestHead {
    /// Create a new request head.
    pub const fn new(
        method: Method,
        path: String,
        version: Version,
        headers: Headers,
    ) -> RequestHead {
        RequestHead {
            method,
            path,
            version,
            headers,
        }
    }

    /// Returns the HTTP method of this request.
    pub const fn method(&self) -> Method {
        self.method
    }

    /// Returns the path of this request.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the HTTP version of this request.
    ///
    /// # Notes
    ///
    /// Requests from the [`HttpServer`] will return the highest version it
    /// understand, e.g. if a client used HTTP/1.2 (which doesn't exists) the
    /// version would be set to HTTP/1.1 (the highest version this crate
    /// understands) per RFC 7230 section 2.6.
    ///
    /// [`HttpServer`]: crate::HttpServer
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Returns the headers.
    pub const fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Returns mutable access to the headers.
    pub const fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Add a body to the request head creating a complete request.
    pub const fn add_body<B>(self, body: B) -> Request<B> {
        Request::from_head(self, body)
    }
}

impl fmt::Debug for RequestHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestHead")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .finish()
    }
}
