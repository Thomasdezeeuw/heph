use std::fmt;

use crate::{Headers, Method, Version};

/// HTTP request.
pub struct Request<B> {
    method: Method,
    path: String,
    version: Version,
    headers: Headers,
    body: B,
}

impl<B> Request<B> {
    /// Create a new request.
    pub const fn new(
        method: Method,
        path: String,
        version: Version,
        headers: Headers,
        body: B,
    ) -> Request<B> {
        Request {
            method,
            version,
            path,
            headers,
            body,
        }
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

    /// Returns the HTTP method of this request.
    pub const fn method(&self) -> Method {
        self.method
    }

    /// Returns the path of this request.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the headers.
    pub const fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Returns mutable access to the headers.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// The request body.
    pub const fn body(&self) -> &B {
        &self.body
    }

    /// Mutable access to the request body.
    pub fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }
}

impl<B> fmt::Debug for Request<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .finish()
    }
}
