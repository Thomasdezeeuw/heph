use std::fmt;

use crate::{Headers, StatusCode, Version};

/// HTTP response.
pub struct Response<B> {
    version: Version,
    status: StatusCode,
    headers: Headers,
    body: B,
}

impl<B> Response<B> {
    /// Create a new HTTP response.
    pub const fn new(
        version: Version,
        status: StatusCode,
        headers: Headers,
        body: B,
    ) -> Response<B> {
        Response {
            version,
            status,
            headers,
            body,
        }
    }

    /// Returns the HTTP version of this response.
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Returns the response code.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the headers.
    pub const fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Returns mutable access to the headers.
    pub const fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Returns a reference to the body.
    pub const fn body(&self) -> &B {
        &self.body
    }

    /// Returns a mutable reference to the body.
    pub const fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }

    /// Returns the body of the response.
    pub fn into_body(self) -> B {
        self.body
    }
}

impl<B> fmt::Debug for Response<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("version", &self.version)
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish()
    }
}
