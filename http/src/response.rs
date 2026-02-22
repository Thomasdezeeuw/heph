use std::ops::{Deref, DerefMut};

use crate::body::EmptyBody;
use crate::head::ResponseHead;
use crate::head::header::{Csv, FromHeaderValue};
use crate::{Header, HeaderName, Headers, Method, StatusCode, Version};

/// HTTP response.
#[derive(Debug)]
pub struct Response<B> {
    head: ResponseHead,
    body: B,
}

impl<B> Response<B> {
    /// Create a new HTTP response.
    ///
    /// See the [builder pattern implementation] below for an easier to use API.
    ///
    /// [builder pattern implementation]: #impl-Response<EmptyBody>
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

    /// Returns the HTTP version of this response.
    pub const fn version(&self) -> Version {
        self.head.version()
    }

    /// Returns a mutable reference to the HTTP version of this response.
    pub const fn version_mut(&mut self) -> &mut Version {
        self.head.version_mut()
    }

    /// Returns the response code.
    pub const fn status(&self) -> StatusCode {
        self.head.status()
    }

    /// Returns a mutable reference to the response code.
    pub const fn status_mut(&mut self) -> &mut StatusCode {
        self.head.status_mut()
    }

    /// Returns the headers.
    pub const fn headers(&self) -> &Headers {
        self.head.headers()
    }

    /// Returns mutable access to the headers.
    pub const fn headers_mut(&mut self) -> &mut Headers {
        self.head.headers_mut()
    }

    /// Get the header’s value with `name`, if any.
    ///
    /// See [`Headers::get_value`] for more information.
    pub fn header<'a, T>(&'a self, name: &HeaderName<'_>) -> Result<Option<T>, T::Err>
    where
        T: FromHeaderValue<'a>,
    {
        self.head.header(name)
    }

    /// Get the header’s value with `name` or return `default`.
    ///
    /// If no header with `name` is found or the [`FromHeaderValue`]
    /// implementation fails this will return `default`. For more control over
    /// the error handling see [`ResponseHead::header`].
    pub fn header_or<'a, T>(&'a self, name: &HeaderName<'_>, default: T) -> T
    where
        T: FromHeaderValue<'a>,
    {
        self.head.header_or(name, default)
    }

    /// Get the header’s value with `name` or returns the result of `default`.
    ///
    /// Same as [`ResponseHead::header_or`] but uses a function to create the
    /// default value.
    pub fn header_or_else<'a, F, T>(&'a self, name: &HeaderName<'_>, default: F) -> T
    where
        T: FromHeaderValue<'a>,
        F: FnOnce() -> T,
    {
        self.head.header_or_else(name, default)
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
            .append_header(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
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
            .append_header(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
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
            .append_header(Header::new(HeaderName::LOCATION, location_uri.as_bytes()));
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
    pub fn method_not_allowed(allowed: &[Method]) -> Response<EmptyBody> {
        let mut response = Response::build_new(StatusCode::METHOD_NOT_ALLOWED);
        // SAFETY: Method never returns an error formatting, so it's safe to
        // unwrap.
        response
            .headers_mut()
            .append(HeaderName::ALLOW, Csv::new(allowed))
            .unwrap();
        response
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
