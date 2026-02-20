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

use std::{fmt, io};

use heph::messages::Terminate;

use crate::process;

mod server;

#[doc(inline)]
pub use a10::net::*;
pub use server::Server;

/// The message type used by server actors.
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`process::Signal`]`>` for the message, allowing for graceful
/// shutdown.
#[derive(Debug)]
pub struct ServerMessage {
    // Allow for future expansion.
    _inner: (),
}

impl From<Terminate> for ServerMessage {
    fn from(_: Terminate) -> ServerMessage {
        ServerMessage { _inner: () }
    }
}

impl TryFrom<process::Signal> for ServerMessage {
    type Error = ();

    fn try_from(signal: process::Signal) -> Result<Self, Self::Error> {
        if signal.should_exit() {
            Ok(ServerMessage { _inner: () })
        } else {
            Err(())
        }
    }
}

/// Error returned by server actors.
#[derive(Debug)]
pub enum ServerError<E> {
    /// Error accepting an incoming connection.
    Accept(io::Error),
    /// Error creating a new actor to handle the connection.
    NewActor(E),
}

impl<E: fmt::Display> fmt::Display for ServerError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::Accept(err) => write!(f, "error accepting connection: {err}"),
            ServerError::NewActor(err) => write!(f, "error creating new actor: {err}"),
        }
    }
}
