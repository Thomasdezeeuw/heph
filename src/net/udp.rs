//! UDP related types.

use std::io;
use std::net::SocketAddr;
use std::task::{LocalWaker, Poll};

use mio_st::net::{ConnectedUdpSocket as MioConnectedUdpSocket, UdpSocket as MioUdpSocket};
use mio_st::poll::PollOption;

use crate::actor::Context;
use crate::net::{interrupted, would_block};

/// A connected User Datagram Protocol (UDP) socket.
///
/// This works much like the [`UdpSocket`] however it can only send to and
/// receive from a single remote address.
#[derive(Debug)]
pub struct ConnectedUdpSocket {
    /// Underlying UDP socket, backed by mio.
    socket: MioConnectedUdpSocket,
}

impl ConnectedUdpSocket {
    /// Connect to a `remote` address, binding to the `local` address.
    ///
    /// This method first binds a UDP socket to the `local` address, then
    /// connects that socket to `remote` address.
    pub fn connect<M>(ctx: &mut Context<M>, local: SocketAddr, remote: SocketAddr) -> io::Result<ConnectedUdpSocket> {
        let mut socket = MioConnectedUdpSocket::connect(local, remote)?;
        let pid = ctx.pid();
        ctx.system_ref().poller_register(&mut socket, pid.into(),
            MioConnectedUdpSocket::INTERESTS, PollOption::Edge)?;
        Ok(ConnectedUdpSocket { socket })
    }

    /// Returns the sockets local address.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Sends data on the socket to the connected socket. On success, returns
    /// the number of bytes written.
    pub fn poll_send(&mut self, _waker: &LocalWaker, buf: &[u8]) -> Poll<io::Result<usize>> {
        try_io!(self.socket.send(buf))
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read.
    pub fn poll_recv(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        try_io!(self.socket.recv(buf))
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// On success, returns the number of bytes read.
    pub fn poll_peek(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        try_io!(self.socket.peek(buf))
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&mut self) -> io::Result<Option<io::Error>> {
        self.socket.take_error()
    }
}

/// An User Datagram Protocol (UDP) socket.
///
/// This works much like the `UdpSocket` in the standard library, but the
/// `send_to`, `recv_from` and `peek_from` methods are now prefixed with `poll_`
/// and work with futures and so they won't block, instead returning
/// `Poll::Pending`.
#[derive(Debug)]
pub struct UdpSocket {
    /// Underlying UDP socket, backed by mio.
    socket: MioUdpSocket,
}

impl UdpSocket {
    /// Create a UDP socket binding to the `address`.
    pub fn bind<M>(ctx: &mut Context<M>, address: SocketAddr) -> io::Result<UdpSocket> {
        let mut socket = MioUdpSocket::bind(address)?;
        let pid = ctx.pid();
        ctx.system_ref().poller_register(&mut socket, pid.into(),
            MioUdpSocket::INTERESTS, PollOption::Edge)?;
        Ok(UdpSocket { socket })
    }

    /// Connects the UDP socket by setting the default destination and limiting
    /// packets that are read, written and peeked to the `remote_address`.
    pub fn connect(self, remote_address: SocketAddr) -> io::Result<ConnectedUdpSocket> {
        self.socket.connect(remote_address)
            .map(|socket| ConnectedUdpSocket { socket })
    }

    /// Returns the sockets local address.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Sends data to the given `target` address. On success, returns the number
    /// of bytes written.
    pub fn poll_send_to(&mut self, _waker: &LocalWaker, buf: &[u8], target: SocketAddr) -> Poll<io::Result<usize>> {
        try_io!(self.socket.send_to(buf, target))
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub fn poll_recv_from(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<(usize, SocketAddr)>> {
        try_io!(self.socket.recv_from(buf))
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// On success, returns the number of bytes read and the address from whence
    /// the data came.
    pub fn poll_peek_from(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<(usize, SocketAddr)>> {
        try_io!(self.socket.peek_from(buf))
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&mut self) -> io::Result<Option<io::Error>> {
        self.socket.take_error()
    }
}
