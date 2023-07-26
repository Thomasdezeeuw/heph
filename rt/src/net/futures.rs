//! A10 I/O [`Future`] wrappers.

#![allow(missing_debug_implementations)]

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};

use a10::extract::Extractor;

use crate::io::{Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::wakers::no_ring_ctx;

/// [`Future`] behind `recv` implementations.
pub(crate) struct Recv<'a, B>(pub(crate) a10::net::Recv<'a, BufWrapper<B>>);

impl<'a, B: BufMut> Future for Recv<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind `recv_n` implementations.
pub(crate) struct RecvN<'a, B>(pub(crate) a10::net::RecvN<'a, BufWrapper<B>>);

impl<'a, B: BufMut> Future for RecvN<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind `recv_vectored` implementations.
pub(crate) struct RecvVectored<'a, B, const N: usize>(
    pub(crate) a10::net::RecvVectored<'a, BufWrapper<B>, N>,
);

impl<'a, B: BufMutSlice<N>, const N: usize> Future for RecvVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, _)| buf.0)
    }
}

/// [`Future`] behind `recv_n_vectored` implementations.
pub(crate) struct RecvNVectored<'a, B, const N: usize>(
    pub(crate) a10::net::RecvNVectored<'a, BufWrapper<B>, N>,
);

impl<'a, B: BufMutSlice<N>, const N: usize> Future for RecvNVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind `recv_from` implementations.
pub(crate) struct RecvFrom<'a, B, A>(pub(crate) a10::net::RecvFrom<'a, BufWrapper<B>, A>);

impl<'a, B: BufMut, A: a10::net::SocketAddress> Future for RecvFrom<'a, B, A> {
    type Output = io::Result<(B, A)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, addr, _)| (buf.0, addr))
    }
}

/// [`Future`] behind `recv_from_vectored` implementations.
pub(crate) struct RecvFromVectored<'a, B, A, const N: usize>(
    pub(crate) a10::net::RecvFromVectored<'a, BufWrapper<B>, A, N>,
);

impl<'a, B: BufMutSlice<N>, A: a10::net::SocketAddress, const N: usize> Future
    for RecvFromVectored<'a, B, A, N>
{
    type Output = io::Result<(B, A)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, addr, _)| (buf.0, addr))
    }
}

/// [`Future`] behind `send` implementations.
pub(crate) struct Send<'a, B>(pub(crate) Extractor<a10::net::Send<'a, BufWrapper<B>>>);

impl<'a, B: Buf> Future for Send<'a, B> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, len)| (buf.0, len))
    }
}

/// [`Future`] behind `send_all` implementations.
pub(crate) struct SendAll<'a, B>(pub(crate) Extractor<a10::net::SendAll<'a, BufWrapper<B>>>);

impl<'a, B: Buf> Future for SendAll<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind `send_vectored` implementations.
pub(crate) struct SendVectored<'a, B, const N: usize>(
    pub(crate) Extractor<a10::net::SendMsg<'a, BufWrapper<B>, a10::net::NoAddress, N>>,
);

impl<'a, B: BufSlice<N>, const N: usize> Future for SendVectored<'a, B, N> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, n)| (buf.0, n))
    }
}

/// [`Future`] behind `send_all_vectored` implementations.
pub(crate) struct SendAllVectored<'a, B, const N: usize>(
    pub(crate) Extractor<a10::net::SendAllVectored<'a, BufWrapper<B>, N>>,
);

impl<'a, B: BufSlice<N>, const N: usize> Future for SendAllVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind `send_to` implementations.
pub(crate) struct SendTo<'a, B, A>(pub(crate) Extractor<a10::net::SendTo<'a, BufWrapper<B>, A>>);

impl<'a, B: Buf, A: a10::net::SocketAddress> Future for SendTo<'a, B, A> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, len)| (buf.0, len))
    }
}

/// [`Future`] behind `send_to_vectored` implementations.
pub(crate) struct SendToVectored<'a, B, A, const N: usize>(
    pub(crate) Extractor<a10::net::SendMsg<'a, BufWrapper<B>, A, N>>,
);

impl<'a, B: BufSlice<N>, A: a10::net::SocketAddress, const N: usize> Future
    for SendToVectored<'a, B, A, N>
{
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, n)| (buf.0, n))
    }
}
