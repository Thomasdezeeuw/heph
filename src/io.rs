//! I/O related trait.
//!
//! Once available these traits will be replaced with the trait from the
//! `futures-io` crate. For docs see the [`futures-io-preview`] crate.
//!
//! For convenience some types from the standard libary's I/O module are
//! re-exported.
//!
//! [`futures-io-preview`]: https://docs.rs/futures-io-preview

pub use std::io::{Error, ErrorKind, Result, Initializer};

use std::task::{Context, Poll};

/// Asynchronous version of the `Read` trait.
pub trait AsyncRead {
    /// See `Read.initializer`.
    #[inline]
    unsafe fn initializer(&self) -> Initializer {
        Initializer::zeroing()
    }

    /// Asynchronous version of the `Read.read` method.
    fn poll_read(&mut self, ctx: &mut Context, buf: &mut [u8]) -> Poll<Result<usize>>;
}

/// Asynchronous version of the `Write` trait.
pub trait AsyncWrite {
    /// Asynchronous version of the `Write.write` method.
    fn poll_write(&mut self, ctx: &mut Context, buf: &[u8]) -> Poll<Result<usize>>;

    /// Asynchronous version of the `Write.flush` method.
    fn poll_flush(&mut self, ctx: &mut Context) -> Poll<Result<()>>;
}
