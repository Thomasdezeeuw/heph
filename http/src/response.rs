use std::fmt;

use crate::{Header, Headers, StatusCode, Version};

pub struct Response<B> {
    pub(crate) version: Version,
    pub(crate) status: StatusCode,
    pub(crate) headers: Headers,
    pub(crate) body: B,
}

impl<B> Response<B> {
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
