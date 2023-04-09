//! Module with [`TcpListener`] and related types.

use std::async_iter::AsyncIterator;
use std::mem::ManuallyDrop;
use std::net::SocketAddr;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use a10::AsyncFd;
use heph::actor;
use mio::Interest;
use socket2::{Domain, Protocol, SockRef, Socket, Type};

use crate::net::{convert_address, SockAddr, TcpStream};
use crate::{self as rt};

/// A TCP socket listener.
///
/// A listener can be created using [`TcpListener::bind`]. After it is created
/// there are two ways to accept incoming [`TcpStream`]s:
///
///  * [`accept`] accepts a single connection, or
///  * [`incoming`] which returns stream of incoming connections.
///
/// [`accept`]: TcpListener::accept
/// [`incoming`]: TcpListener::incoming
///
/// # Examples
///
/// Accepting a single [`TcpStream`], using [`TcpListener::accept`].
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use log::error;
///
/// use heph::{actor, SupervisorStrategy};
/// # use heph_rt::net::TcpStream;
/// use heph_rt::net::TcpListener;
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
/// use log::info;
///
/// fn main() -> Result<(), rt::Error> {
///     std_logger::Config::logfmt().init();
///
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:8000".parse().unwrap();
///
///     runtime_ref.spawn_local(supervisor, actor as fn(_, _) -> _, address, ActorOptions::default());
/// #   runtime_ref.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default());
///
///     Ok(())
/// }
/// #
/// # async fn client(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let mut buf = Vec::with_capacity(local_address.len() + 1);
/// #   stream.recv_n(&mut buf, local_address.len()).await?;
/// #   assert_eq!(buf, local_address.as_bytes());
/// #   Ok(())
/// # }
///
/// // Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///
///     // Accept a connection.
///     let (unbound_stream, peer_address) = listener.accept().await?;
///     info!("accepted connection from: {peer_address}");
///
///     // Next we need to bind the stream to this actor.
///     let mut stream = unbound_stream.bind_to(&mut ctx)?;
///
///     // Next we write the IP address to the connection.
///     let ip = peer_address.to_string();
///     stream.send_all(ip.as_bytes()).await
/// }
/// ```
///
/// Accepting multiple [`TcpStream`]s, using [`TcpListener::incoming`].
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use log::{error, info};
///
/// use heph::{actor, SupervisorStrategy};
/// use heph_rt::net::TcpListener;
/// # use heph_rt::net::TcpStream;
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
/// use heph_rt::util::next;
///
/// fn main() -> Result<(), rt::Error> {
///     std_logger::Config::logfmt().init();
///
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:8000".parse().unwrap();
///
///     runtime_ref.spawn_local(supervisor, actor as fn(_, _) -> _, address, ActorOptions::default());
/// #   runtime_ref.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default());
///
///     Ok(())
/// }
/// #
/// # async fn client(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let mut buf = Vec::with_capacity(local_address.len() + 1);
/// #   stream.recv_n(&mut buf, local_address.len()).await?;
/// #   assert_eq!(buf, local_address.as_bytes());
/// #   Ok(())
/// # }
///
/// // Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///     let mut incoming = listener.incoming();
///     loop {
///         let unbound_stream = match next(&mut incoming).await {
///             Some(Ok(unbound_stream)) => unbound_stream,
///             Some(Err(err)) => return Err(err),
///             None => return Ok(()),
///         };
///
///         let mut stream = unbound_stream.bind_to(&mut ctx)?;
///         let peer_address = stream.peer_addr()?;
///         info!("accepted connection from: {peer_address}");
///
///         // Next we write the IP address to the connection.
///         let ip = peer_address.to_string();
///         stream.send_all(ip.as_bytes()).await?;
/// #       return Ok(());
///     }
/// }
/// ```
pub struct TcpListener {
    fd: AsyncFd,
}

impl TcpListener {
    /// Creates a new `TcpListener` which will be bound to the specified
    /// `address`.
    pub async fn bind<RT>(rt: &RT, address: SocketAddr) -> io::Result<TcpListener>
    where
        RT: rt::Access,
    {
        TcpListener::bind_setup(rt, address, |_| Ok(())).await
    }

    pub(crate) async fn bind_setup<RT, F>(
        rt: &RT,
        address: SocketAddr,
        setup: F,
    ) -> io::Result<TcpListener>
    where
        RT: rt::Access,
        F: FnOnce(&Socket) -> io::Result<()>,
    {
        let fd = a10::net::socket(
            rt.submission_queue(),
            Domain::for_address(address).into(),
            Type::STREAM.cloexec().into(),
            Protocol::TCP.into(),
            0,
        )
        .await?;

        let socket = TcpListener { fd };

        socket.with_ref(|socket| {
            #[cfg(target_os = "linux")]
            if let Some(cpu) = rt.cpu() {
                if let Err(err) = socket.set_cpu_affinity(cpu) {
                    log::warn!("failed to set CPU affinity on UdpSocket: {err}");
                }
            }

            setup(&socket)?;
            socket.bind(&address.into())?;
            socket.listen(1024)?;

            Ok(())
        })?;

        Ok(socket)
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.with_ref(|socket| socket.local_addr().and_then(convert_address))
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.with_ref(|socket| socket.set_ttl(ttl))
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.with_ref(|socket| socket.ttl())
    }

    /// Accept a new incoming [`TcpStream`].
    ///
    /// Returns the TCP stream and the remote address of the peer. See the
    /// [`TcpListener`] documentation for an example.
    pub async fn accept(&mut self) -> io::Result<(UnboundTcpStream, SocketAddr)> {
        self.fd
            .accept::<SockAddr>()
            .await
            .map(|(fd, addr)| (UnboundTcpStream::from_async_fd(fd), addr.into()))
    }

    /// Returns a stream of incoming [`TcpStream`]s.
    ///
    /// Note that unlike [`accept`] this doesn't return the address because uses
    /// io_uring's multishot accept (making it faster then calling `accept` in a
    /// loop). See the [`TcpListener`] documentation for an example.
    ///
    /// [`accept`]: TcpListener::accept
    pub fn incoming(&mut self) -> Incoming<'_> {
        Incoming(self.fd.multishot_accept())
    }

    /// Temp function used by `TcpListener`.
    pub(crate) fn incoming2(&mut self) -> a10::net::MultishotAccept<'_> {
        self.fd.multishot_accept()
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&mut self) -> io::Result<Option<io::Error>> {
        self.with_ref(|socket| socket.take_error())
    }

    pub(crate) fn with_ref<F, T>(&self, f: F) -> io::Result<T>
    where
        F: FnOnce(SockRef<'_>) -> io::Result<T>,
    {
        let borrowed = self.fd.as_fd(); // TODO: remove this once we update to socket2 v0.5.
        f(SockRef::from(&borrowed))
    }
}

/// An unbound [`TcpStream`].
///
/// The stream first has to be bound to an actor (using [`bind_to`]), before it
/// can be used.
///
/// [`bind_to`]: UnboundTcpStream::bind_to
#[derive(Debug)]
pub struct UnboundTcpStream {
    stream: TcpStream,
}

impl UnboundTcpStream {
    /// Bind this TCP stream to the actor's `ctx`, allowing it to be used.
    pub fn bind_to<M, RT>(mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<TcpStream>
    where
        RT: rt::Access,
    {
        let mut stream = ctx
            .runtime()
            .register(
                &mut self.stream.socket,
                Interest::READABLE | Interest::WRITABLE,
            )
            .map(|()| self.stream)?;
        #[cfg(target_os = "linux")]
        if let Some(cpu) = ctx.runtime_ref().cpu() {
            if let Err(err) = stream.set_cpu_affinity(cpu) {
                log::warn!("failed to set CPU affinity on TcpStream: {err}");
            }
        }
        Ok(stream)
    }

    pub(crate) fn from_async_fd(fd: AsyncFd) -> UnboundTcpStream {
        UnboundTcpStream {
            stream: TcpStream {
                // SAFETY: the put `fd` in a `ManuallyDrop` to ensure we don't
                // close it, so we're free to create a `TcpStream` from the fd.
                socket: unsafe {
                    let fd = ManuallyDrop::new(fd);
                    FromRawFd::from_raw_fd(fd.as_fd().as_raw_fd())
                },
            },
        }
    }
}

/// The [`AsyncIterator`] behind [`TcpListener::incoming`].
#[derive(Debug)]
#[must_use = "AsyncIterators do nothing unless polled"]
pub struct Incoming<'a>(a10::net::MultishotAccept<'a>);

impl<'a> AsyncIterator for Incoming<'a> {
    type Item = io::Result<UnboundTcpStream>;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll_next(ctx)
            .map_ok(UnboundTcpStream::from_async_fd)
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
