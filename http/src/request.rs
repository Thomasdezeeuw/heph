// TODO: see if we can have a borrowed version of `Request`.

use std::fmt;

use crate::{Header, Headers, Method, Version};

pub struct Request<B> {
    pub(crate) method: Method,
    pub(crate) path: String,
    pub(crate) version: Version,
    pub(crate) headers: Headers,
    pub(crate) body: B,
}

impl<B> Request<B> {
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
    pub fn version(&self) -> Version {
        self.version
    }

    /// Returns the HTTP method of this request.
    pub fn method(&self) -> Method {
        self.method
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    pub fn body(&self) -> &B {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }

    // TODO: maybe `fn split_body(self) -> (Request<()>, B)`?
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
