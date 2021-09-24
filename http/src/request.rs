use std::fmt;
use std::ops::{Deref, DerefMut};

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

impl<B> fmt::Debug for Request<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request").field("head", &self.head).finish()
    }
}
