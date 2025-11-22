//! Cancelation of I/O operations on [`AsyncFd`].
//!
//! See the [`Cancel`] trait to cancel a specific operation or
//! [`AsyncFd::cancel_all`] to cancel all operations on a fd.
//!
//! [`AsyncFd`]: crate::fd::AsyncFd
//! [`AsyncFd::cancel_all`]: crate::fd::AsyncFd::cancel_all

pub use a10::cancel::*;
