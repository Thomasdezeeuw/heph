//! Module with [`TcpStream`] and related types.

use std::future::Future;
use std::io::{self, IoSlice, IoSliceMut, Read, Write};
use std::mem::{size_of, MaybeUninit};
use std::net::{Shutdown, SocketAddr};
use std::ops::DerefMut;
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::task::{self, Poll};

use futures_io::{AsyncRead, AsyncWrite};
use mio::{net, Interest};

use crate::actor;
use crate::rt::RuntimeAccess;

/// A non-blocking TCP stream between a local socket and a remote socket.
#[derive(Debug)]
pub struct TcpStream {
    /// Underlying TCP connection, backed by Mio.
    pub(in crate::net) socket: net::TcpStream,
}

impl TcpStream {
    /// Create a new TCP stream and issues a non-blocking connect to the
    /// specified `address`.
    ///
    /// # Notes
    ///
    /// The stream is also [bound] to the actor that owns the `actor::Context`,
    /// which means the actor will be run every time the stream is ready to read
    /// or write.
    ///
    /// [bound]: crate::actor::Bound
    pub fn connect<M, K>(ctx: &mut actor::Context<M, K>, address: SocketAddr) -> io::Result<Connect>
    where
        K: RuntimeAccess,
    {
        let mut socket = net::TcpStream::connect(address)?;
        let pid = ctx.pid();
        ctx.kind().register(
            &mut socket,
            pid.into(),
            Interest::READABLE | Interest::WRITABLE,
        )?;
        Ok(Connect {
            socket: Some(socket),
        })
    }

    /// Returns the socket address of the remote peer of this TCP connection.
    pub fn peer_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.peer_addr()
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.socket.set_ttl(ttl)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.socket.ttl()
    }

    /// Sets the value of the `TCP_NODELAY` option on this socket.
    pub fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.socket.set_nodelay(nodelay)
    }

    /// Gets the value of the `TCP_NODELAY` option on this socket.
    pub fn nodelay(&mut self) -> io::Result<bool> {
        self.socket.nodelay()
    }

    /// Returns `true` if `SO_KEEPALIVE` is set.
    pub fn keepalive(&self) -> io::Result<bool> {
        // NOTE: this belongs in the socket2 crate.
        let mut keepalive: libc::c_int = -1;
        let mut keepalive_size = size_of::<libc::c_int>() as libc::socklen_t;
        syscall!(getsockopt(
            self.socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            &mut keepalive as *mut _ as *mut _,
            &mut keepalive_size,
        ))
        .map(|_| {
            debug_assert_eq!(keepalive_size as usize, size_of::<libc::c_int>());
            keepalive != 0
        })
    }

    /// Enables or disables `SO_KEEPALIVE`.
    pub fn set_keepalive(&self, enable: bool) -> io::Result<()> {
        // NOTE: this belongs in the socket2 crate.
        let enable = enable as libc::c_int;
        syscall!(setsockopt(
            self.socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            &enable as *const _ as *const _,
            size_of::<libc::c_int>() as libc::socklen_t,
        ))
        .map(|_| ())
    }

    /// Attempt to receive message(s) from the stream, writing them into `buf`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`TcpStream::recv`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        syscall!(recv(
            self.socket.as_raw_fd(),
            MaybeUninit::slice_as_mut_ptr(buf).cast(),
            buf.len(),
            0, // Flags.
        ))
        .map(|read| read as usize)
    }

    /// Receive messages from the stream, writing them into `buf`.
    pub fn recv<'a>(&'a mut self, buf: &'a mut [MaybeUninit<u8>]) -> Recv<'a> {
        Recv { stream: self, buf }
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified
    /// portions to return immediately with an appropriate value (see the
    /// documentation of [`Shutdown`]).
    pub fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        self.socket.shutdown(how)
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

/// The [`Future`] behind [`TcpStream::connect`].
#[derive(Debug)]
pub struct Connect {
    socket: Option<net::TcpStream>,
}

impl Future for Connect {
    type Output = io::Result<TcpStream>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // This relates directly Mio and `kqueue(2)` and `epoll(2)`. To do a
        // non-blocking TCP connect properly we need to a couple of this.
        //
        // 1. Setup a socket and call `connect(2)`. Mio does this for us.
        //    However it doesn't mean the socket is connected, as we can't
        //    determine that without blocking.
        // 2. To determine if a socket is connected we need to wait for a
        //    `kqueue(2)`/`epoll(2)` event (we get scheduled once we do). But
        //    that doesn't tell us whether or not the socket is connected or
        //    not. To determine if the socket is connected we need to use
        //    `getpeername` (`TcpStream::peer_addr`). But before checking if
        //    we're connected we need to check for a connection error, by
        //    checking `SO_ERROR` (`TcpStream::take_error`) to not loose that
        //    information.
        //    However if we get an event (and thus get scheduled) and
        //    `getpeername` fails it doesn't actually mean the socket will never
        //    connect properly. So we loop (by returned `Poll::Pending`) until
        //    either `SO_ERROR` is set or the socket is connected.
        //
        // Sources:
        // * https://cr.yp.to/docs/connect.html
        // * https://stackoverflow.com/questions/17769964/linux-sockets-non-blocking-connect
        match self.socket.take() {
            Some(socket) => {
                // If we hit an error while connecting return that.
                if let Ok(Some(err)) = socket.take_error() {
                    return Poll::Ready(Err(err));
                }

                // If we can get a peer address it means the stream is
                // connected.
                if let Ok(..) = socket.peer_addr() {
                    return Poll::Ready(Ok(TcpStream { socket }));
                }

                self.socket = Some(socket);
                Poll::Pending
            }
            None => panic!("polled `tcp::stream::Connect` after completion"),
        }
    }
}

/// The [`Future`] behind [`TcpStream::recv`].
#[derive(Debug)]
pub struct Recv<'a> {
    stream: &'a mut TcpStream,
    buf: &'a mut [MaybeUninit<u8>],
}

impl<'a> Future for Recv<'a> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Recv {
            ref mut stream,
            ref mut buf,
        } = self.deref_mut();
        try_io!(stream.try_recv(buf))
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        try_io!(self.socket.read(buf))
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        try_io!(self.socket.read_vectored(bufs))
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        try_io!(self.socket.write(buf))
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        try_io!(self.socket.write_vectored(bufs))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        try_io!(self.socket.flush())
    }

    fn poll_close(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(ctx)
    }
}

impl<K> actor::Bound<K> for TcpStream
where
    K: RuntimeAccess,
{
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, K>) -> io::Result<()> {
        let pid = ctx.pid();
        ctx.kind().reregister(
            &mut self.socket,
            pid.into(),
            Interest::READABLE | Interest::WRITABLE,
        )
    }
}
