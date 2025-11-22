//! Filesystem manipulation operations.
//!
//! To open a file ([`AsyncFd`]) use [`open_file`] or [`OpenOptions`].
//!
//! [`AsyncFd`]: crate::fd::AsyncFd

pub mod watch;
#[doc(no_inline)]
pub use watch::Watch;

#[doc(inline)]
pub use a10::fs::*;
