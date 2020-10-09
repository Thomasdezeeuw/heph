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
//! [`UdpSocket`]: crate::net::UdpSocket

/// A macro to try an I/O function.
///
/// Note that this is used in the tcp and udp modules and has to be defined
/// before them, otherwise this would have been place below.
macro_rules! try_io {
    ($op: expr) => {
        loop {
            match $op {
                Ok(ok) => break Poll::Ready(Ok(ok)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    };
}

// TODO: better name than rpc.
pub mod rpc;
pub mod tcp;
pub mod udp;

#[doc(no_inline)]
pub use tcp::{TcpListener, TcpServer, TcpStream};
#[doc(no_inline)]
pub use udp::UdpSocket;

#[cfg(test)]
mod tests;
