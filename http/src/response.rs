use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::body::EmptyBody;
use crate::head::ResponseHead;
use crate::{Header, HeaderName, Headers, StatusCode, Version};

/// HTTP response.
pub struct Response<B> {
    head: ResponseHead,
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
        Response::from_head(ResponseHead::new(version, status, headers), body)
    }

    /// Create a new response from a [`ResponseHead`] and a `body`.
    pub const fn from_head(head: ResponseHead, body: B) -> Response<B> {
        Response { head, body }
    }

    /// Returns a reference to the body.
    pub const fn body(&self) -> &B {
        &self.body
    }

    /// Returns a mutable reference to the body.
    pub const fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }

    /// Map the body from type `B` to `U` by calling `map`.
    pub fn map_body<F, U>(self, map: F) -> Response<U>
    where
        F: FnOnce(B) -> U,
    {
        Response {
            head: self.head,
            body: map(self.body),
        }
    }

    /// Split the response in the head and the body.
    // TODO: mark as constant once destructors (that this doesn't have) can be
    // run in constant functions.
    pub fn split(self) -> (ResponseHead, B) {
        (self.head, self.body)
    }
}

/// Builder pattern for `Response`.
///
/// These methods create a new `Response` using HTTP/1.1, no headers and an
/// empty body. Use [`Response::with_body`] to add a body to the response.
impl Response<EmptyBody> {
    /// Create a 200 OK response.
    pub const fn ok() -> Response<EmptyBody> {
        Response::build_new(StatusCode::OK)
    }

    /// Create a 201 Created response.
    pub const fn created() -> Response<EmptyBody> {
        Response::build_new(StatusCode::CREATED)
    }

    /// Create a 204 No Content response.
    pub const fn no_content() -> Response<EmptyBody> {
        Response::build_new(StatusCode::NO_CONTENT)
    }

    /// Create a 301 Moved Permanently response.
    ///
    /// Sets the [Location](HeaderName::LOCATION) header to `location_uri`.
    pub fn moved(location_uri: &str) -> Response<EmptyBody> {
        let mut response = Response::build_new(StatusCode::MOVED_PERMANENTLY);
        response
            .head
            .headers_mut()
            .append(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
        response
    }

    /// Create a 302 Found response.
    ///
    /// Sets the [Location](HeaderName::LOCATION) header to `location_uri`.
    pub fn found(location_uri: &str) -> Response<EmptyBody> {
        let mut response = Response::build_new(StatusCode::FOUND);
        response
            .head
            .headers_mut()
            .append(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
        response
    }

    /// Create a 303 See Other response.
    ///
    /// Sets the [Location](HeaderName::LOCATION) header to `location_uri`.
    pub fn see_other(location_uri: &str) -> Response<EmptyBody> {
        let mut response = Response::build_new(StatusCode::SEE_OTHER);
        response
            .head
            .headers_mut()
            .append(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
        response
    }

    /// Create a 304 Not Modified response.
    pub const fn not_modified() -> Response<EmptyBody> {
        Response::build_new(StatusCode::NOT_MODIFIED)
    }

    /// Create a 400 Bad Request response.
    pub const fn bad_request() -> Response<EmptyBody> {
        Response::build_new(StatusCode::BAD_REQUEST)
    }

    /// Create a 401 Unauthorized response.
    pub const fn unauthorized() -> Response<EmptyBody> {
        Response::build_new(StatusCode::UNAUTHORIZED)
    }

    /// Create a 403 Forbidden response.
    pub const fn forbidden() -> Response<EmptyBody> {
        Response::build_new(StatusCode::FORBIDDEN)
    }

    /// Create a 404 Not Found response.
    pub const fn not_found() -> Response<EmptyBody> {
        Response::build_new(StatusCode::NOT_FOUND)
    }

    /// Create a 405 Method Not Allow response.
    pub const fn method_not_allowed() -> Response<EmptyBody> {
        Response::build_new(StatusCode::METHOD_NOT_ALLOWED)
        // TODO: set Allow header.
    }

    /// Create a 410 Gone response.
    pub const fn gone() -> Response<EmptyBody> {
        Response::build_new(StatusCode::GONE)
    }

    /// Create a 411 Length required response.
    pub const fn length_required() -> Response<EmptyBody> {
        Response::build_new(StatusCode::LENGTH_REQUIRED)
    }

    /// Create a 500 Internal Server Error response.
    pub const fn server_error() -> Response<EmptyBody> {
        Response::build_new(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Create a 501 Not Implemented response.
    pub const fn not_implemented() -> Response<EmptyBody> {
        Response::build_new(StatusCode::NOT_IMPLEMENTED)
    }

    /// Create a 502 Bad Gateway response.
    pub const fn bad_gateway() -> Response<EmptyBody> {
        Response::build_new(StatusCode::BAD_GATEWAY)
    }

    /// Create a 503 Service Unavailable response.
    pub const fn unavailable() -> Response<EmptyBody> {
        Response::build_new(StatusCode::SERVICE_UNAVAILABLE)
    }

    /// Create a 504 Gateway Timeout response.
    pub const fn gateway_timeout() -> Response<EmptyBody> {
        Response::build_new(StatusCode::GATEWAY_TIMEOUT)
    }

    /// Simple version of [`Response::new`] used by the build functions.
    pub(crate) const fn build_new(status: StatusCode) -> Response<EmptyBody> {
        Response::new(Version::Http11, status, Headers::EMPTY, EmptyBody)
    }

    /// Add a body to the response.
    pub fn with_body<B>(self, body: B) -> Response<B> {
        Response {
            head: self.head,
            body,
        }
    }
}

impl<B> Deref for Response<B> {
    type Target = ResponseHead;

    fn deref(&self) -> &Self::Target {
        &self.head
    }
}

impl<B> DerefMut for Response<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.head
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<B> fmt::Debug for Response<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("head", &self.head)
            .finish()
    }
}
