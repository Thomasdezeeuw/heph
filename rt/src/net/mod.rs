//! Networking primitives.
//!
//! To create a new socket ([`AsyncFd`]) use the [`socket`] function, which
//! issues a non-blocking `socket(2)` call.
//!
//! [`AsyncFd`]: crate::fd::AsyncFd

use std::{fmt, io};

use heph::messages::Terminate;

use crate::Signal;

mod tcp_server;

#[doc(inline)]
pub use a10::net::{
    socket, Accept, AcceptFlag, Bind, Connect, Domain, IPv4Opt, IPv6Opt, Level, Listen,
    MultishotAccept, MultishotRecv, NoAddress, Opt, Protocol, Recv, RecvFlag, RecvFrom,
    RecvFromVectored, RecvN, RecvNVectored, RecvVectored, Send, SendAll, SendAllVectored, SendFlag,
    SendMsg, SendTo, SetSocketOption, Shutdown, Socket, SocketAddress, SocketOpt, SocketOption,
    TcpOpt, Type, UdpOpt, UnixOpt,
};
pub use tcp_server::TcpServer;

/// The message type used by server actors.
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`Signal`]`>` for the message, allowing for graceful shutdown.
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

impl TryFrom<Signal> for ServerMessage {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => {
                Ok(ServerMessage { _inner: () })
            }
            _ => Err(()),
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
