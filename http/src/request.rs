use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::body::EmptyBody;
use crate::head::header::{FromHeaderValue, HeaderName};
use crate::head::{Path, RequestHead};
use crate::{Headers, Method, Version};

/// HTTP request.
pub struct Request<B> {
    head: RequestHead,
    body: B,
}

impl<B> Request<B> {
    /// Create a new request.
    pub const fn new(
        method: Method,
        path: Path,
        version: Version,
        headers: Headers,
        body: B,
    ) -> Request<B> {
        Request::from_head(RequestHead::new(method, path, version, headers), body)
    }

    /// Create a new request from a [`RequestHead`] and a `body`.
    pub const fn from_head(head: RequestHead, body: B) -> Request<B> {
        Request { head, body }
    }

    /// Returns the HTTP method of this request.
    pub const fn method(&self) -> Method {
        self.head.method()
    }

    /// Returns a mutable reference to the HTTP method of this request.
    pub const fn method_mut(&mut self) -> &mut Method {
        self.head.method_mut()
    }

    /// Returns the path of this request.
    pub fn path(&self) -> &str {
        self.head.path()
    }

    /// Returns a mutable reference to the path of this request.
    pub fn path_mut(&mut self) -> &mut Path {
        self.head.path_mut()
    }

    /// Returns the HTTP version of this request.
    ///
    /// # Notes
    ///
    /// Requests from the HTTP server will return the highest version it
    /// understands, e.g. if a client used HTTP/1.2 (which doesn't exists) the
    /// version would be set to HTTP/1.1 (the highest version this crate
    /// understands) per RFC 9110 section 6.2.
    pub const fn version(&self) -> Version {
        self.head.version()
    }

    /// Returns a mutable reference to the HTTP version of this request.
    pub const fn version_mut(&mut self) -> &mut Version {
        self.head.version_mut()
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
    /// the error handling see [`RequestHead::header`].
    pub fn header_or<'a, T>(&'a self, name: &HeaderName<'_>, default: T) -> T
    where
        T: FromHeaderValue<'a>,
    {
        self.head.header_or(name, default)
    }

    /// Get the header’s value with `name` or returns the result of `default`.
    ///
    /// Same as [`RequestHead::header_or`] but uses a function to create the
    /// default value.
    pub fn header_or_else<'a, F, T>(&'a self, name: &HeaderName<'_>, default: F) -> T
    where
        T: FromHeaderValue<'a>,
        F: FnOnce() -> T,
    {
        self.head.header_or_else(name, default)
    }

    /// The request body.
    pub const fn body(&self) -> &B {
        &self.body
    }

    /// Mutable access to the request body.
    pub const fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }

    /// Map the body from type `B` to `U` by calling `map`.
    pub fn map_body<F, U>(self, map: F) -> Request<U>
    where
        F: FnOnce(B) -> U,
    {
        Request {
            head: self.head,
            body: map(self.body),
        }
    }

    /// Split the request in the head and the body.
    // TODO: mark as constant once destructors (that this doesn't have) can be
    // run in constant functions.
    pub fn split(self) -> (RequestHead, B) {
        (self.head, self.body)
    }
}

/// Builder pattern for `Request`.
///
/// These methods create a new `Request` using HTTP/1.1, no headers and an empty
/// body. Use [`Request::with_body`] to add a body to the request.
impl Request<EmptyBody> {
    /// Create a GET request.
    pub const fn get(path: Path) -> Request<EmptyBody> {
        Request::build_new(Method::Get, path)
    }

    /// Create a HEAD request.
    pub const fn head(path: Path) -> Request<EmptyBody> {
        Request::build_new(Method::Head, path)
    }

    /// Create a POST request.
    pub const fn post(path: Path) -> Request<EmptyBody> {
        Request::build_new(Method::Post, path)
    }

    /// Create a PUT request.
    pub const fn put(path: Path) -> Request<EmptyBody> {
        Request::build_new(Method::Put, path)
    }

    /// Create a DELETE request.
    pub const fn delete(path: Path) -> Request<EmptyBody> {
        Request::build_new(Method::Delete, path)
    }

    /// Simple version of [`Request::new`] used by the build functions.
    const fn build_new(method: Method, path: Path) -> Request<EmptyBody> {
        Request::new(method, path, Version::Http11, Headers::EMPTY, EmptyBody)
    }

    /// Add a body to the request.
    pub fn with_body<B>(self, body: B) -> Request<B> {
        Request {
            head: self.head,
            body,
        }
    }
}

impl<B> Deref for Request<B> {
    type Target = RequestHead;

    fn deref(&self) -> &Self::Target {
        &self.head
    }
}

impl<B> DerefMut for Request<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.head
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<B> fmt::Debug for Request<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request").field("head", &self.head).finish()
    }
}
