//! Unix Domain Sockets (UDS).
//!
//! Heph supports two types of UDS sockets:
//! * Stream-oriented sockets, similar to [TCP], which has two main types:
//!   * A [Unix stream] to send and receive data.
//!   * A [Unix listening socket], a socket used to listen for connections.
//! * Datagram-oriented sockets, similar to [UDP], only provides a single
//!   socket type:
//!   * [`UnixDatagram`].
//!
//! [TCP]: crate::net::tcp
//! [Unix stream]: crate::net::uds::UnixStream
//! [Unix listening socket]: crate::net::uds::UnixListener
//! [UDP]: crate::net::udp
//! [`UnixDatagram`]: crate::unix::uds::UnixDatagram

pub mod datagram;

#[doc(no_inline)]
pub use datagram::UnixDatagram;
