use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::head::ResponseHead;
use crate::{Headers, StatusCode, Version};

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

    /// Create a new request from a [`ResponseHead`] and a `body`.
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

impl<B> fmt::Debug for Response<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("head", &self.head)
            .finish()
    }
}
