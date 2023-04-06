//! User Datagram Protocol (UDP) related types.
//!
//! See [`UdpSocket`].

// TODO: a number of send/recv methods don't use Mio directly, this is fine on
// Unix but doesn't work on Windows (which we don't support). We need to fix
// that once Mio uses Socket2 and supports all the methods we need, Mio's
// tracking issue: https://github.com/tokio-rs/mio/issues/1381.

use std::fmt;
use std::future::Future;
use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor;
#[cfg(target_os = "linux")]
use log::warn;
use mio::{net, Interest};
use socket2::{SockAddr, SockRef};

use crate::bytes::{Bytes, BytesVectored, MaybeUninitSlice};
use crate::net::convert_address;
use crate::{self as rt, Bound};

pub use crate::net::{Connected, Unconnected};

/// A User Datagram Protocol (UDP) socket.
///
/// To create a UDP socket [`UdpSocket::bind`] can be used, this will bind the
/// socket to a local address. The created socket will be in unconnected mode. A
/// socket can be in one of two modes:
///
/// - [`Unconnected`] mode allows sending and receiving packets to and from all
///   sources.
/// - [`Connected`] mode only allows sending and receiving packets from/to a
///   single source.
///
/// An unconnected socket can be [connected] to a specific address if needed,
/// changing the mode to [`Connected`] in the process. The remote address of an
/// already connected socket can be changed to a different address using the
/// same method.
///
/// Both unconnected and connected sockets have three main operations send,
/// receive and peek, all these methods return a [`Future`].
///
/// [connected]: UdpSocket::connect
///
/// # Examples
///
/// ```
/// #![feature(never_type)]
///
/// use std::net::SocketAddr;
/// use std::{io, str};
///
/// use log::error;
///
/// use heph::messages::Terminate;
/// use heph::{actor, SupervisorStrategy};
/// use heph_rt::net::UdpSocket;
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::util::either;
/// use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
///
/// fn main() -> Result<(), rt::Error> {
///     std_logger::Config::logfmt().init();
///
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// fn setup(mut runtime: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:7000".parse().unwrap();
///     // Add our server actor.
///     runtime.spawn_local(supervisor, echo_server as fn(_, _) -> _, address, ActorOptions::default());
///     // Add our client actor.
///     runtime.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default());
///     Ok(())
/// }
///
/// /// Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// /// Actor that will bind a UDP socket and waits for incoming packets and
/// /// echos the message to standard out.
/// async fn echo_server(mut ctx: actor::Context<Terminate, ThreadLocal>, local: SocketAddr) -> io::Result<()> {
///     let mut socket = UdpSocket::bind(&mut ctx, local)?;
///     let mut buf = Vec::with_capacity(4096);
///     loop {
///         buf.clear();
///         let receive_msg = ctx.receive_next();
///         let read = socket.recv_from(&mut buf);
///         let address = match either(read, receive_msg).await {
///             // Received a packet.
///             Ok(Ok((_, address))) => address,
///             // Read error.
///             Ok(Err(err)) => return Err(err),
///             // If we receive a terminate message we'll stop the actor.
///             Err(_) => return Ok(()),
///         };
///
///         match str::from_utf8(&buf) {
///             Ok(str) => println!("Got the following message: `{str}`, from {address}"),
///             Err(_) => println!("Got data: {buf:?}, from {address}"),
///         }
/// #       return Ok(());
///     }
/// }
///
/// /// The client that will send a message to the server.
/// async fn client(mut ctx: actor::Context<!, ThreadLocal>, server_address: SocketAddr) -> io::Result<()> {
///     let local_address = "127.0.0.1:7001".parse().unwrap();
///     let mut socket = UdpSocket::bind(&mut ctx, local_address)
///         .and_then(|socket| socket.connect(server_address))?;
///
///     let msg = b"Hello world";
///     let n = socket.send(&*msg).await?;
///     assert_eq!(n, msg.len());
///     Ok(())
/// }
/// ```
pub struct UdpSocket<M = Unconnected> {
    /// Underlying UDP socket, backed by Mio.
    socket: net::UdpSocket,
    /// The mode in which the socket is in, this determines what methods are
    /// available.
    mode: PhantomData<M>,
}

impl UdpSocket {
    /// Create a UDP socket binding to the `local` address.
    ///
    /// # Notes
    ///
    /// The UDP socket is also [bound] to the actor that owns the
    /// `actor::Context`, which means the actor will be run every time the
    /// socket is ready to be read from or write to.
    ///
    /// [bound]: crate::Bound
    pub fn bind<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        local: SocketAddr,
    ) -> io::Result<UdpSocket<Unconnected>>
    where
        RT: rt::Access,
    {
        let mut socket = net::UdpSocket::bind(local)?;
        ctx.runtime()
            .register(&mut socket, Interest::READABLE | Interest::WRITABLE)?;
        #[cfg(target_os = "linux")]
        if let Some(cpu) = ctx.runtime_ref().cpu() {
            if let Err(err) = SockRef::from(&socket).set_cpu_affinity(cpu) {
                warn!("failed to set CPU affinity on UdpSocket: {err}");
            }
        }
        Ok(UdpSocket {
            socket,
            mode: PhantomData,
        })
    }
}

impl<M> UdpSocket<M> {
    /// Connects the UDP socket by setting the default destination and limiting
    /// packets that are read, written and peeked to the `remote` address.
    pub fn connect(self, remote: SocketAddr) -> io::Result<UdpSocket<Connected>> {
        self.socket.connect(remote).map(|()| UdpSocket {
            socket: self.socket,
            mode: PhantomData,
        })
    }

    /// Returns the sockets local address.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
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

impl UdpSocket<Unconnected> {
    /// Attempt to send data to the given `target` address.
    ///
    /// If the buffer currently can't be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::send_to`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send_to(&mut self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        self.socket.send_to(buf, target)
    }

    /// Sends data to the given `target` address. Returns a [`Future`] that on
    /// success returns the number of bytes written (`io::Result<usize>`).
    pub fn send_to<'a, 'b>(&'a mut self, buf: &'b [u8], target: SocketAddr) -> SendTo<'a, 'b> {
        SendTo {
            socket: self,
            buf,
            target,
        }
    }

    /// Attempt to send bytes in `bufs` to the peer.
    ///
    /// If no bytes can currently be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::send_to_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send_to_vectored(
        &mut self,
        bufs: &[IoSlice<'_>],
        target: SocketAddr,
    ) -> io::Result<usize> {
        SockRef::from(&self.socket).send_to_vectored(bufs, &target.into())
    }

    /// Send the bytes in `bufs` to the peer.
    ///
    /// Returns the number of bytes written. This may be fewer then the length
    /// of `bufs`.
    pub fn send_to_vectored<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
        target: SocketAddr,
    ) -> SendToVectored<'a, 'b> {
        SendToVectored {
            socket: self,
            bufs,
            target: target.into(),
        }
    }

    /// Attempt to receive data from the socket, writing them into `buf`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv_from`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv_from<B>(&mut self, mut buf: B) -> io::Result<(usize, SocketAddr)>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `UdpSocket::try_recv_from` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .recv_from(buf.as_bytes())
            .and_then(|(read, address)| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                let address = convert_address(address)?;
                Ok((read, address))
            })
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read and the address from whence the data
    /// came (`io::Result<(usize, SocketAddr>`).
    pub fn recv_from<B>(&mut self, buf: B) -> RecvFrom<'_, B>
    where
        B: Bytes,
    {
        RecvFrom { socket: self, buf }
    }

    /// Attempt to receive data from the socket, writing them into `bufs`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv_from`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv_from_vectored<B>(&mut self, mut bufs: B) -> io::Result<(usize, SocketAddr)>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `UdpSocket::try_recv_from` with empty buffers"
        );
        let res = SockRef::from(&self.socket)
            .recv_from_vectored(MaybeUninitSlice::as_socket2(bufs.as_bufs().as_mut()));
        match res {
            Ok((read, _, address)) => {
                // Safety: just read the bytes.
                unsafe { bufs.update_lengths(read) }
                let address = convert_address(address)?;
                Ok((read, address))
            }
            Err(err) => Err(err),
        }
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read and the address from whence the data
    /// came (`io::Result<(usize, SocketAddr>`).
    pub fn recv_from_vectored<B>(&mut self, bufs: B) -> RecvFromVectored<'_, B>
    where
        B: BytesVectored,
    {
        RecvFromVectored { socket: self, bufs }
    }

    /// Attempt to peek data from the socket, writing them into `buf`.
    ///
    /// If no bytes can currently be peeked this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::peek_from`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_peek_from<B>(&mut self, mut buf: B) -> io::Result<(usize, SocketAddr)>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `UdpSocket::try_peek_from` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .peek_from(buf.as_bytes())
            .and_then(|(read, address)| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                let address = convert_address(address)?;
                Ok((read, address))
            })
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// and the address from whence the data came (`io::Result<(usize,
    /// SocketAddr>`).
    pub fn peek_from<B>(&mut self, buf: B) -> PeekFrom<'_, B>
    where
        B: Bytes,
    {
        PeekFrom { socket: self, buf }
    }

    /// Attempt to peek data from the socket, writing them into `bufs`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_peek_from_vectored<B>(&mut self, mut bufs: B) -> io::Result<(usize, SocketAddr)>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `UdpSocket::try_peek_from_vectored` with empty buffers"
        );
        let res = SockRef::from(&self.socket).recv_from_vectored_with_flags(
            MaybeUninitSlice::as_socket2(bufs.as_bufs().as_mut()),
            libc::MSG_PEEK,
        );
        match res {
            Ok((read, _, address)) => {
                // Safety: just read the bytes.
                unsafe { bufs.update_lengths(read) }
                let address = convert_address(address)?;
                Ok((read, address))
            }
            Err(err) => Err(err),
        }
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// and the address from whence the data came (`io::Result<(usize,
    /// SocketAddr>`).
    pub fn peek_from_vectored<B>(&mut self, bufs: B) -> PeekFromVectored<'_, B>
    where
        B: BytesVectored,
    {
        PeekFromVectored { socket: self, bufs }
    }
}

/// The [`Future`] behind [`UdpSocket::send_to`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendTo<'a, 'b> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: &'b [u8],
    target: SocketAddr,
}

impl<'a, 'b> Future for SendTo<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        #[rustfmt::skip]
        let SendTo { socket, buf, target } = Pin::into_inner(self);
        try_io!(socket.try_send_to(buf, *target))
    }
}

/// The [`Future`] behind [`UdpSocket::send_to_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendToVectored<'a, 'b> {
    socket: &'a mut UdpSocket<Unconnected>,
    bufs: &'b mut [IoSlice<'b>],
    target: SockAddr,
}

impl<'a, 'b> Future for SendToVectored<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        #[rustfmt::skip]
        let SendToVectored { socket, bufs, target } = Pin::into_inner(self);
        try_io!(SockRef::from(&socket.socket).send_to_vectored(bufs, target))
    }
}

/// The [`Future`] behind [`UdpSocket::recv_from`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvFrom<'a, B> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: B,
}

impl<'a, B> Future for RecvFrom<'a, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvFrom { socket, buf } = Pin::into_inner(self);
        try_io!(socket.try_recv_from(&mut *buf))
    }
}

/// The [`Future`] behind [`UdpSocket::recv_from_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvFromVectored<'a, B> {
    socket: &'a mut UdpSocket<Unconnected>,
    bufs: B,
}

impl<'a, B> Future for RecvFromVectored<'a, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvFromVectored { socket, bufs } = Pin::into_inner(self);
        try_io!(socket.try_recv_from_vectored(&mut *bufs))
    }
}

/// The [`Future`] behind [`UdpSocket::peek_from`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PeekFrom<'a, B> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: B,
}

impl<'a, B> Future for PeekFrom<'a, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekFrom { socket, buf } = Pin::into_inner(self);
        try_io!(socket.try_peek_from(&mut *buf))
    }
}

/// The [`Future`] behind [`UdpSocket::peek_from_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PeekFromVectored<'a, B> {
    socket: &'a mut UdpSocket<Unconnected>,
    bufs: B,
}

impl<'a, B> Future for PeekFromVectored<'a, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekFromVectored { socket, bufs } = Pin::into_inner(self);
        try_io!(socket.try_peek_from_vectored(&mut *bufs))
    }
}

impl UdpSocket<Connected> {
    /// Attempt to send data to the peer.
    ///
    /// If the buffer currently can't be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::send_to`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.socket.send(buf)
    }

    /// Sends data on the socket to the connected socket. Returns a [`Future`]
    /// that on success returns the number of bytes written
    /// (`io::Result<usize>`).
    pub fn send<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Send<'a, 'b> {
        Send { socket: self, buf }
    }

    /// Attempt to send bytes in `bufs` to the peer.
    ///
    /// If no bytes can currently be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::send_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        SockRef::from(&self.socket).send_vectored(bufs)
    }

    /// Send the bytes in `bufs` to the peer.
    ///
    /// Returns the number of bytes written. This may we fewer then the length
    /// of `bufs`.
    pub fn send_vectored<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> SendVectored<'a, 'b> {
        SendVectored { socket: self, bufs }
    }

    /// Attempt to receive data from the socket, writing them into `buf`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv<B>(&mut self, mut buf: B) -> io::Result<usize>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `UdpSocket::try_recv` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .recv(buf.as_bytes())
            .map(|read| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                read
            })
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read (`io::Result<usize>`).
    pub fn recv<B>(&mut self, buf: B) -> Recv<'_, B>
    where
        B: Bytes,
    {
        Recv { socket: self, buf }
    }

    /// Attempt to receive data from the socket, writing them into `bufs`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv_vectored<B>(&mut self, mut bufs: B) -> io::Result<usize>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `UdpSocket::try_recv_vectored` with empty buffers"
        );
        let res = SockRef::from(&self.socket)
            .recv_vectored(MaybeUninitSlice::as_socket2(bufs.as_bufs().as_mut()));
        match res {
            Ok((read, _)) => {
                // Safety: just read the bytes.
                unsafe { bufs.update_lengths(read) }
                Ok(read)
            }
            Err(err) => Err(err),
        }
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read (`io::Result<usize>`).
    pub fn recv_vectored<B>(&mut self, bufs: B) -> RecvVectored<'_, B>
    where
        B: BytesVectored,
    {
        RecvVectored { socket: self, bufs }
    }

    /// Attempt to peek data from the socket, writing them into `buf`.
    ///
    /// If no bytes can currently be peeked this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::peek`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_peek<B>(&mut self, mut buf: B) -> io::Result<usize>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `UdpSocket::try_peek` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .peek(buf.as_bytes())
            .map(|read| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                read
            })
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// (`io::Result<usize>`).
    pub fn peek<B>(&mut self, buf: B) -> Peek<'_, B>
    where
        B: Bytes,
    {
        Peek { socket: self, buf }
    }

    /// Attempt to peek data from the socket, writing them into `bufs`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`UdpSocket::recv_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_peek_vectored<B>(&mut self, mut bufs: B) -> io::Result<usize>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `UdpSocket::try_peek_vectored` with empty buffers"
        );
        let res = SockRef::from(&self.socket).recv_vectored_with_flags(
            MaybeUninitSlice::as_socket2(bufs.as_bufs().as_mut()),
            libc::MSG_PEEK,
        );
        match res {
            Ok((read, _)) => {
                // Safety: just read the bytes.
                unsafe { bufs.update_lengths(read) }
                Ok(read)
            }
            Err(err) => Err(err),
        }
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// (`io::Result<usize>`).
    pub fn peek_vectored<B>(&mut self, bufs: B) -> PeekVectored<'_, B>
    where
        B: BytesVectored,
    {
        PeekVectored { socket: self, bufs }
    }
}

/// The [`Future`] behind [`UdpSocket::send`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Send<'a, 'b> {
    socket: &'a mut UdpSocket<Connected>,
    buf: &'b [u8],
}

impl<'a, 'b> Future for Send<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Send { socket, buf } = Pin::into_inner(self);
        try_io!(socket.try_send(buf))
    }
}

/// The [`Future`] behind [`UdpSocket::send_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendVectored<'a, 'b> {
    socket: &'a mut UdpSocket<Connected>,
    bufs: &'b mut [IoSlice<'b>],
}

impl<'a, 'b> Future for SendVectored<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let SendVectored { socket, bufs } = Pin::into_inner(self);
        try_io!(socket.try_send_vectored(bufs))
    }
}

/// The [`Future`] behind [`UdpSocket::recv`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Recv<'a, B> {
    socket: &'a mut UdpSocket<Connected>,
    buf: B,
}

impl<'a, B> Future for Recv<'a, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Recv { socket, buf } = Pin::into_inner(self);
        try_io!(socket.try_recv(&mut *buf))
    }
}

/// The [`Future`] behind [`UdpSocket::recv_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PeekVectored<'a, B> {
    socket: &'a mut UdpSocket<Connected>,
    bufs: B,
}

impl<'a, B> Future for PeekVectored<'a, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekVectored { socket, bufs } = Pin::into_inner(self);
        try_io!(socket.try_peek_vectored(&mut *bufs))
    }
}

/// The [`Future`] behind [`UdpSocket::peek`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Peek<'a, B> {
    socket: &'a mut UdpSocket<Connected>,
    buf: B,
}

impl<'a, B> Future for Peek<'a, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Peek { socket, buf } = Pin::into_inner(self);
        try_io!(socket.try_peek(&mut *buf))
    }
}

/// The [`Future`] behind [`UdpSocket::recv_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvVectored<'a, B> {
    socket: &'a mut UdpSocket<Connected>,
    bufs: B,
}

impl<'a, B> Future for RecvVectored<'a, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvVectored { socket, bufs } = Pin::into_inner(self);
        try_io!(socket.try_recv_vectored(&mut *bufs))
    }
}

impl<M> fmt::Debug for UdpSocket<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.socket.fmt(f)
    }
}

impl<M, RT: rt::Access> Bound<RT> for UdpSocket<M> {
    type Error = io::Error;

    fn bind_to<Msg>(&mut self, ctx: &mut actor::Context<Msg, RT>) -> io::Result<()> {
        ctx.runtime()
            .reregister(&mut self.socket, Interest::READABLE | Interest::WRITABLE)
    }
}
