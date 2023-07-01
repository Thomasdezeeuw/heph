use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::body::EmptyBody;
use crate::head::RequestHead;
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
        path: String,
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
    pub const fn get(path: String) -> Request<EmptyBody> {
        Request::build_new(Method::Get, path)
    }

    /// Create a HEAD request.
    pub const fn head(path: String) -> Request<EmptyBody> {
        Request::build_new(Method::Head, path)
    }

    /// Create a POST request.
    pub const fn post(path: String) -> Request<EmptyBody> {
        Request::build_new(Method::Post, path)
    }

    /// Create a PUT request.
    pub const fn put(path: String) -> Request<EmptyBody> {
        Request::build_new(Method::Put, path)
    }

    /// Create a DELETE request.
    pub const fn delete(path: String) -> Request<EmptyBody> {
        Request::build_new(Method::Delete, path)
    }

    /// Simple version of [`Request::new`] used by the build functions.
    const fn build_new(method: Method, path: String) -> Request<EmptyBody> {
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
