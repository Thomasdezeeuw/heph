//! Network related types.
//!
//! The network module support two types of protocols:
//!
//! * [Transmission Control Protocol] (TCP) module provides three main types:
//!   * A [TCP stream] between a local and a remote socket.
//!   * A [TCP listening socket], a socket used to listen for connections.
//!   * A [TCP server], listens for connections and starts a new actor for each.
//! * [User Datagram Protocol] (UDP) only provides a single socket type:
//!   * [`UdpSocket`].
//!
//! [Transmission Control Protocol]: crate::net::tcp
//! [TCP stream]: crate::net::TcpStream
//! [TCP listening socket]: crate::net::TcpListener
//! [TCP server]: crate::net::TcpServer
//! [User Datagram Protocol]: crate::net::udp
//!
//! # I/O with Heph's socket
//!
//! The different socket types provide two or three variants of most I/O
//! functions. The `try_*` funtions, which makes the system calls once. For
//! example [`TcpStream::try_send`] calls `send(2)` once, not handling any
//! errors (including [`WouldBlock`] errors!).
//!
//! In addition they provide a [`Future`] function which handles would block
//! errors. For `TcpStream::try_send` the future version is [`TcpStream::send`],
//! i.e. without the `try_` prefix.
//!
//! Finally for a lot of function a convenience version is provided that handle
//! various cases. For example with sending you might want to ensure all bytes
//! are send, for this you can use [`TcpStream::send_all`]. But also see
//! functions such as [`TcpStream::recv_n`]; which receives at least `n` bytes,
//! or [`TcpStream::send_entire_file`]; which sends an entire file using the
//! `sendfile(2)` system call.
//!
//! [`WouldBlock`]: io::ErrorKind::WouldBlock
//! [`Future`]: std::future::Future
//!
//! # Notes
//!
//! All types in the `net` module around [bound] to an actor. See the
//! [`actor::Bound`] trait for more information.
//!
//! [bound]: crate::actor::Bound
//! [`actor::Bound`]: crate::actor::Bound

use std::io;
use std::net::SocketAddr;

use socket2::SockAddr;

pub mod tcp;
pub mod udp;

#[doc(no_inline)]
pub use tcp::{TcpListener, TcpServer, TcpStream};
#[doc(no_inline)]
pub use udp::UdpSocket;
/// Convert a `socket2:::SockAddr` into a `std::net::SocketAddr`.
#[allow(clippy::needless_pass_by_value)]
fn convert_address(address: SockAddr) -> io::Result<SocketAddr> {
    match address.as_socket() {
        Some(address) => Ok(address),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid address family (not IPv4 or IPv6)",
        )),
    }
}
