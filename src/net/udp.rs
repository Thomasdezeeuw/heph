//! User Datagram Protocol (UDP) related types.
//!
//! See [`UdpSocket`].

use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use mio::{net, Interest};
use socket2::SockRef;

use crate::actor;
use crate::net::{convert_address, Bytes};
use crate::rt::{self, PrivateAccess};

/// The unconnected mode of an [`UdpSocket`].
#[allow(missing_debug_implementations)]
pub enum Unconnected {}

/// The connected mode of an [`UdpSocket`].
#[allow(missing_debug_implementations)]
pub enum Connected {}

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
/// use futures_util::future::FutureExt;
/// use futures_util::select;
/// use log::error;
///
/// use heph::actor::messages::Terminate;
/// use heph::net::UdpSocket;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef, SupervisorStrategy};
///
/// fn main() -> Result<(), rt::Error> {
///     heph::log::init();
///
///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
/// }
///
/// fn setup(mut runtime: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:7000".parse().unwrap();
///
///     // Add our server actor.
///     runtime.spawn_local(supervisor, echo_server as fn(_, _) -> _, address,
///         ActorOptions::default());
///
///     // Add our client actor.
///     runtime.spawn_local(supervisor, client as fn(_, _) -> _, address,
///         ActorOptions::default());
///
///     Ok(())
/// }
///
/// // Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// // Actor that will bind a UDP socket and waits for incoming packets and
/// // echos the message to standard out.
/// async fn echo_server(mut ctx: actor::Context<Terminate>, local: SocketAddr) -> io::Result<()> {
///     let mut socket = UdpSocket::bind(&mut ctx, local)?;
///     let mut buf = Vec::with_capacity(4096);
///     loop {
///         buf.clear();
///         let mut receive_msg = ctx.receive_next().fuse();
///         let mut read = socket.recv_from(&mut buf).fuse();
///         let (n, address) = select! {
///             // If we receive a terminate message we'll stop the actor.
///             _ = receive_msg => return Ok(()),
///             // Or we received an packet.
///             res = read => res?,
///         };
///
///         let buf = &buf[.. n];
///         match str::from_utf8(buf) {
///             Ok(str) => println!("Got the following message: `{}`, from {}", str, address),
///             Err(_) => println!("Got data: {:?}, from {}", buf, address),
///         }
/// #       return Ok(());
///     }
/// }
///
/// // The client that will send a message to the server.
/// async fn client(mut ctx: actor::Context<!>, server_address: SocketAddr) -> io::Result<()> {
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
    /// socket is ready to be read or write to.
    ///
    /// [bound]: crate::actor::Bound
    pub fn bind<M, K>(
        ctx: &mut actor::Context<M, K>,
        local: SocketAddr,
    ) -> io::Result<UdpSocket<Unconnected>>
    where
        actor::Context<M, K>: rt::Access,
    {
        let mut socket = net::UdpSocket::bind(local)?;
        let pid = ctx.pid();
        ctx.register(
            &mut socket,
            pid.into(),
            Interest::READABLE | Interest::WRITABLE,
        )?;
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
        let dst = buf.as_bytes();
        debug_assert!(
            !dst.is_empty(),
            "called `UdpSocket::try_recv_from` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .recv_from(dst)
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
        let dst = buf.as_bytes();
        debug_assert!(
            !dst.is_empty(),
            "called `UdpSocket::try_peek_from` with an empty buffer"
        );
        SockRef::from(&self.socket)
            .peek_from(dst)
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
        let SendTo {
            socket,
            buf,
            target,
        } = Pin::into_inner(self);
        try_io!(socket.try_send_to(buf, *target))
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
        let dst = buf.as_bytes();
        debug_assert!(
            !dst.is_empty(),
            "called `UdpSocket::try_recv` with an empty buffer"
        );
        SockRef::from(&self.socket).recv(dst).map(|read| {
            // Safety: just read the bytes.
            unsafe { buf.update_length(read) }
            read
        })
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read (`io::Result<usize>`).
    pub fn recv<B>(&mut self, buf: B) -> Recv<'_, B> {
        Recv { socket: self, buf }
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
        let dst = buf.as_bytes();
        debug_assert!(
            !dst.is_empty(),
            "called `UdpSocket::try_peek` with an empty buffer"
        );
        SockRef::from(&self.socket).peek(dst).map(|read| {
            // Safety: just read the bytes.
            unsafe { buf.update_length(read) }
            read
        })
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// (`io::Result<usize>`).
    pub fn peek<B>(&mut self, buf: B) -> Peek<'_, B> {
        Peek { socket: self, buf }
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

impl<M> fmt::Debug for UdpSocket<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.socket.fmt(f)
    }
}

impl<K> actor::Bound<K> for UdpSocket {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, K>) -> io::Result<()>
    where
        actor::Context<M, K>: rt::Access,
    {
        ctx.reregister(
            &mut self.socket,
            ctx.pid().into(),
            Interest::READABLE | Interest::WRITABLE,
        )
    }
}
