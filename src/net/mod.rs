//! Network related types.
//!
//! # Transmission Control Protocol (TCP)
//!
//! For TCP two types are provided: a listener and a stream (connection).
//!
//! The [`TcpListener`] listens for incoming connections and accepts them
//! starting a new actor for each accepted connection. `TcpListener` implements
//! the [`Initiator`] trait which allows it to be [added] to the actor system.
//!
//! [`TcpStream`] on the other hand represent a single TCP connection. It can
//! either be a connection accepted by the `TcpListener`, or one started by
//! [connecting] to a remote address. It implements the `AsyncRead` and
//! `AsyncWrite` trait to work nicely with futures.
//!
//! [`TcpListener`]: struct.TcpListener.html
//! [`Initiator`]: ../initiator/trait.Initiator.html
//! [added]: ../system/struct.ActorSystem.html#method.with_initiator
//! [`TcpStream`]: struct.TcpStream.html
//! [connecting]: struct.TcpStream.html#method.connect
//!
//! # User Datagram Protocol (UDP)
//!
//! Two UDP socket types are provided: connected and unconnected. If you're
//! familiar with UDP you know that UDP doesn't have a concept of a connection,
//! you send packet across the internet and hope it end up at its destination
//! without many guarantees. However in some cases a back and forth
//! communication is still required, in those cases the [`ConnectedUdpSocket`]
//! type can be used.
//!
//! `ConnectedUdpSocket` is a socket that is bound to a local address and is
//! connected to a remote address. This allows it use [`poll_send`] and
//! [`poll_recv`] rather then [`poll_send_to`] in which you also need to
//! specific the address to send the packet to, this makes it easier to use.
//!
//! However UDP sockets don't need to be connected to a single remote address.
//! It can also be used to receive from and send to a number of different remote
//! addresses. Then [`UdpSocket`] can be used, this is a UDP socket that only
//! bound to a local address. It has the [`poll_send_to`] method that allows
//! sending to any remote address and [`poll_recv_from`] to receive packets from
//! any address.
//!
//! [`ConnectedUdpSocket`]: struct.ConnectedUdpSocket.html
//! [`poll_send`]: struct.ConnectedUdpSocket.html#method.poll_send
//! [`poll_recv`]: struct.ConnectedUdpSocket.html#method.poll_recv
//! [`poll_send_to`]: struct.UdpSocket.html#method.poll_send_to
//! [`UdpSocket`]: struct.UdpSocket.html
//! [`poll_recv_from`]: struct.UdpSocket.html#method.poll_recv_from

use std::io;

mod tcp;
mod udp;

pub use self::tcp::{TcpStream, TcpListener};
pub use self::udp::{ConnectedUdpSocket, UdpSocket};

/// Whether or not the error is a would block error.
pub(crate) fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

/// Whether or not the error is an interrupted error.
pub(crate) fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}
