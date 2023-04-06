//! A10 I/O [`Future`] wrappers.

#![allow(missing_debug_implementations)]

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};

use a10::extract::Extractor;

use crate::io::{Buf, BufMut, BufWrapper};

/// [`Future`] behind send implementations.
pub struct Send<'a, B>(pub(crate) Extractor<a10::net::Send<'a, BufWrapper<B>>>);

impl<'a, B: Buf> Future for Send<'a, B> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, len)| (buf.0, len))
    }
}

/// [`Future`] behind send_to implementations.
pub struct SendTo<'a, B, A>(pub(crate) Extractor<a10::net::SendTo<'a, BufWrapper<B>, A>>);

impl<'a, B: Buf, A: a10::net::SocketAddress> Future for SendTo<'a, B, A> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, _, len)| (buf.0, len))
    }
}

/// [`Future`] behind recv implementations.
pub struct Recv<'a, B>(pub(crate) a10::net::Recv<'a, BufWrapper<B>>);

impl<'a, B: BufMut> Future for Recv<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}
