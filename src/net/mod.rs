//! Network related types.
//!
//! Two types of socket as provided:
//!
//! * [Transmission Control Protocol] (TCP) module provides two main types:
//!   * A [TCP stream] between a local and a remote socket.
//!   * A [TCP socket server], listening for connections.
//! * [User Datagram Protocol] (UDP) only provides a single socket type:
//!   * [`UdpSocket`].
//!
//! [Transmission Control Protocol]: crate::net::tcp
//! [TCP stream]: crate::net::TcpStream
//! [TCP socket server]: crate::net::TcpListener
//! [User Datagram Protocol]: crate::net::udp
//! [`UdpSocket`]: crate::net::UdpSocket

/// A macro to try an I/O function.
///
/// Note that this is used in the tcp and udp modules and has to be defined
/// before them, otherwise this would have been place below.
macro_rules! try_io {
    ($op:expr) => {
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

pub mod tcp;
pub mod udp;

#[doc(no_inline)]
pub use self::tcp::{TcpListener, TcpStream};
#[doc(no_inline)]
pub use self::udp::UdpSocket;
