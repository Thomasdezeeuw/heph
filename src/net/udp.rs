//! User Datagram Protocol (UDP) related types.
//!
//! See [`UdpSocket`].

use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::ops::DerefMut;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use mio::{net, Interest};

use crate::actor;

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
/// use heph::system::RuntimeError;
/// use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef, SupervisorStrategy};
///
/// fn main() -> Result<(), RuntimeError> {
///     heph::log::init();
///
///     ActorSystem::new().with_setup(setup).run()
/// }
///
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
///     let address = "127.0.0.1:7000".parse().unwrap();
///
///     // Add our server actor.
///     system_ref.spawn(supervisor, echo_server as fn(_, _) -> _, address,
///         ActorOptions::default().schedule());
///
///     // Add our client actor.
///     system_ref.spawn(supervisor, client as fn(_, _) -> _, address,
///         ActorOptions::default().schedule());
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
///     let mut buf = [0; 4096];
///     loop {
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
    pub fn bind<M>(
        ctx: &mut actor::Context<M>,
        local: SocketAddr,
    ) -> io::Result<UdpSocket<Unconnected>> {
        let mut socket = net::UdpSocket::bind(local)?;
        let pid = ctx.pid();
        ctx.system_ref().register(
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
        self.socket.connect(remote)?;
        Ok(UdpSocket {
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
    /// Sends data to the given `target` address. Returns a [`Future`] that on
    /// success returns the number of bytes written (`io::Result<usize>`).
    pub fn send_to<'a>(&'a mut self, buf: &'a [u8], target: SocketAddr) -> SendTo<'a> {
        SendTo {
            socket: self,
            buf,
            target,
        }
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read and the address from whence the data
    /// came (`io::Result<(usize, SocketAddr>`).
    pub fn recv_from<'a>(&'a mut self, buf: &'a mut [u8]) -> RecvFrom<'a> {
        RecvFrom { socket: self, buf }
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// and the address from whence the data came (`io::Result<(usize,
    /// SocketAddr>`).
    pub fn peek_from<'a>(&'a mut self, buf: &'a mut [u8]) -> PeekFrom<'a> {
        PeekFrom { socket: self, buf }
    }
}

/// The [`Future`] behind [`UdpSocket::send_to`].
#[derive(Debug)]
pub struct SendTo<'a> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: &'a [u8],
    target: SocketAddr,
}

impl<'a> Future for SendTo<'a> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let SendTo {
            ref mut socket,
            ref buf,
            ref target,
        } = self.deref_mut();
        try_io!(socket.socket.send_to(buf, *target))
    }
}

/// The [`Future`] behind [`UdpSocket::recv_from`].
#[derive(Debug)]
pub struct RecvFrom<'a> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: &'a mut [u8],
}

impl<'a> Future for RecvFrom<'a> {
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvFrom {
            ref mut socket,
            ref mut buf,
        } = self.deref_mut();
        try_io!(socket.socket.recv_from(buf))
    }
}

/// The [`Future`] behind [`UdpSocket::peek_from`].
#[derive(Debug)]
pub struct PeekFrom<'a> {
    socket: &'a mut UdpSocket<Unconnected>,
    buf: &'a mut [u8],
}

impl<'a> Future for PeekFrom<'a> {
    type Output = io::Result<(usize, SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekFrom {
            ref mut socket,
            ref mut buf,
        } = self.deref_mut();
        try_io!(socket.socket.peek_from(buf))
    }
}

impl UdpSocket<Connected> {
    /// Sends data on the socket to the connected socket. Returns a [`Future`]
    /// that on success returns the number of bytes written
    /// (`io::Result<usize>`).
    pub fn send<'a>(&'a mut self, buf: &'a [u8]) -> Send<'a> {
        Send { socket: self, buf }
    }

    /// Receives data from the socket. Returns a [`Future`] that on success
    /// returns the number of bytes read (`io::Result<usize>`).
    pub fn recv<'a>(&'a mut self, buf: &'a mut [u8]) -> Recv<'a> {
        Recv { socket: self, buf }
    }

    /// Receives data from the socket, without removing it from the input queue.
    /// Returns a [`Future`] that on success returns the number of bytes read
    /// (`io::Result<usize>`).
    pub fn peek<'a>(&'a mut self, buf: &'a mut [u8]) -> Peek<'a> {
        Peek { socket: self, buf }
    }
}

/// The [`Future`] behind [`UdpSocket::send`].
#[derive(Debug)]
pub struct Send<'a> {
    socket: &'a mut UdpSocket<Connected>,
    buf: &'a [u8],
}

impl<'a> Future for Send<'a> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Send {
            ref mut socket,
            ref buf,
        } = self.deref_mut();
        try_io!(socket.socket.send(buf))
    }
}

/// The [`Future`] behind [`UdpSocket::recv`].
#[derive(Debug)]
pub struct Recv<'a> {
    socket: &'a mut UdpSocket<Connected>,
    buf: &'a mut [u8],
}

impl<'a> Future for Recv<'a> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Recv {
            ref mut socket,
            ref mut buf,
        } = self.deref_mut();
        try_io!(socket.socket.recv(buf))
    }
}

/// The [`Future`] behind [`UdpSocket::peek`].
#[derive(Debug)]
pub struct Peek<'a> {
    socket: &'a mut UdpSocket<Connected>,
    buf: &'a mut [u8],
}

impl<'a> Future for Peek<'a> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Peek {
            ref mut socket,
            ref mut buf,
        } = self.deref_mut();
        try_io!(socket.socket.peek(buf))
    }
}

impl<M> fmt::Debug for UdpSocket<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.socket.fmt(f)
    }
}

impl actor::Bound for UdpSocket {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M>) -> io::Result<()> {
        let pid = ctx.pid();
        ctx.system_ref().reregister(
            &mut self.socket,
            pid.into(),
            Interest::READABLE | Interest::WRITABLE,
        )
    }
}
