//! Asynchronous file descriptor (fd).
//!
//! See [`AsyncFd`].

#[doc(inline)]
pub use a10::fd::AsyncFd;

#[doc(inline)]
#[cfg(any(target_os = "android", target_os = "linux"))]
pub use a10::fd::{ToDirect, ToFd};
