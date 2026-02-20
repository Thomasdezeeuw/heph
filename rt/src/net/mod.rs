//! Networking primitives.
//!
//! # Creating Sockets
//!
//! Unlike the standard library Heph doesn't use types for different kinds of
//! sockets, we only use [`AsyncFd`]. To create a new socket use the [`socket`]
//! function, followed by [`AsyncFd::bind`] & [`AsyncFd::listen`] (for a
//! TCP/Unix listener) or [`AsyncFd::connect`] (for a TCP/Unix stream or
//! UdpSocket).
//!
//! # Servers
//!
//! Alternatively Heph provides some easy to use servers, such as [`Server`],
//! which handle setting up the listeners and spawns a new actor for each
//! incoming connection.
//!
//! [`AsyncFd`]: crate::fd::AsyncFd
//! [`AsyncFd::bind`]: crate::fd::AsyncFd::bind
//! [`AsyncFd::listen`]: crate::fd::AsyncFd::listen
//! [`AsyncFd::connect`]: crate::fd::AsyncFd::connect

mod server;

#[doc(inline)]
pub use a10::net::*;
pub use server::{Server, ServerError, ServerMessage};
