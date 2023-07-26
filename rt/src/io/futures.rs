//! A10 I/O [`Future`] wrappers.

#![allow(missing_debug_implementations)]

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};

use a10::extract::Extractor;

use crate::io::buf::{Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::wakers::no_ring_ctx;

/// [`Future`] behind write implementations.
pub(crate) struct Write<'a, B>(pub(crate) Extractor<a10::io::Write<'a, BufWrapper<B>>>);

impl<'a, B: Buf> Future for Write<'a, B> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, len)| (buf.0, len))
    }
}

/// [`Future`] behind write all implementations.
pub(crate) struct WriteAll<'a, B>(pub(crate) Extractor<a10::io::WriteAll<'a, BufWrapper<B>>>);

impl<'a, B: Buf> Future for WriteAll<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind write vectored implementations.
pub(crate) struct WriteVectored<'a, B, const N: usize>(
    pub(crate) Extractor<a10::io::WriteVectored<'a, BufWrapper<B>, N>>,
);

impl<'a, B: BufSlice<N>, const N: usize> Future for WriteVectored<'a, B, N> {
    type Output = io::Result<(B, usize)>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|(buf, len)| (buf.0, len))
    }
}

/// [`Future`] behind write all vectored implementations.
pub(crate) struct WriteAllVectored<'a, B, const N: usize>(
    pub(crate) Extractor<a10::io::WriteAllVectored<'a, BufWrapper<B>, N>>,
);

impl<'a, B: BufSlice<N>, const N: usize> Future for WriteAllVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind read implementations.
pub(crate) struct Read<'a, B>(pub(crate) a10::io::Read<'a, BufWrapper<B>>);

impl<'a, B: BufMut> Future for Read<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind read vectored implementations.
pub(crate) struct ReadVectored<'a, B, const N: usize>(
    pub(crate) a10::io::ReadVectored<'a, BufWrapper<B>, N>,
);

impl<'a, B: BufMutSlice<N>, const N: usize> Future for ReadVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind read `n` implementations.
pub(crate) struct ReadN<'a, B>(pub(crate) a10::io::ReadN<'a, BufWrapper<B>>);

impl<'a, B: BufMut> Future for ReadN<'a, B> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}

/// [`Future`] behind read `n` vectored implementations.
pub(crate) struct ReadNVectored<'a, B, const N: usize>(
    pub(crate) a10::io::ReadNVectored<'a, BufWrapper<B>, N>,
);

impl<'a, B: BufMutSlice<N>, const N: usize> Future for ReadNVectored<'a, B, N> {
    type Output = io::Result<B>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll(ctx)
            .map_ok(|buf| buf.0)
    }
}
