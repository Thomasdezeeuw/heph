//! Network related types.
//!
//! # Transmission Control Protocol (TCP)
//!
//! For TCP two types are provided: a listener and a stream (connection).
//!
//! The [`TcpListener`] listens for incoming connections and accepts them
//! starting a new actor for each accepted connection.
//!
//! [`TcpStream`] on the other hand represent a single TCP connection. It can
//! either be a connection accepted by the `TcpListener`, or one started by
//! [connecting] to a remote address. It implements the `AsyncRead` and
//! `AsyncWrite` trait to work nicely with futures.
//!
//! [`TcpListener`]: crate::net::TcpListener
//! [connecting]: crate::net::TcpStream::connect
//! [`TcpStream`]: crate::net::TcpStream
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
//! [`ConnectedUdpSocket`]: crate::net::ConnectedUdpSocket
//! [`poll_send`]: crate::net::ConnectedUdpSocket::poll_send
//! [`poll_recv`]: crate::net::ConnectedUdpSocket::poll_recv
//! [`poll_send_to`]: crate::net::UdpSocket::poll_send_to
//! [`UdpSocket`]: crate::net::UdpSocket
//! [`poll_recv_from`]: crate::net::UdpSocket::poll_recv_from

use std::io;

/// A macro to try an I/O function.
///
/// Note that this is used in the tcp and udp modules and has to be defined
/// before them, otherwise this would have been place below.
macro_rules! try_io {
    ($op:expr) => {
        loop {
            match $op {
                Ok(ok) => return Poll::Ready(Ok(ok)),
                Err(ref err) if would_block(err) => return Poll::Pending,
                Err(ref err) if interrupted(err) => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    };
}

mod tcp;
pub mod udp;

pub use self::tcp::{TcpListener, TcpListenerError, TcpListenerMessage, TcpStream};
#[doc(no_inline)]
pub use self::udp::UdpSocket;

/// Whether or not the error is a would block error.
pub(crate) fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

/// Whether or not the error is an interrupted error.
pub(crate) fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}
