//! Module with [`TcpListener`] and related types.

use std::async_iter::AsyncIterator;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use a10::AsyncFd;
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
/// # async fn client(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(ctx.runtime_ref(), address).await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let buf = Vec::with_capacity(local_address.len() + 1);
/// #   let buf = stream.recv_n(buf, local_address.len()).await?;
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
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///
///     // Accept a connection.
///     let (mut stream, peer_address) = listener.accept().await?;
///     info!("accepted connection from: {peer_address}");
///
///     // Next we write the IP address to the connection.
///     let ip = peer_address.to_string();
///     stream.send_all(ip).await?;
///     Ok(())
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
/// # async fn client(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(ctx.runtime_ref(), address).await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let buf = Vec::with_capacity(local_address.len() + 1);
/// #   let buf = stream.recv_n(buf, local_address.len()).await?;
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
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///     let mut incoming = listener.incoming();
///     loop {
///         let mut stream = match next(&mut incoming).await {
///             Some(Ok(stream)) => stream,
///             Some(Err(err)) => return Err(err),
///             None => return Ok(()),
///         };
///
///         // Optionally set the CPU affinity as that's not done automatically
///         // (in case the stream is send to another thread).
///         stream.set_auto_cpu_affinity(ctx.runtime_ref());
///
///         let peer_address = stream.peer_addr()?;
///         info!("accepted connection from: {peer_address}");
///
///         // Next we write the IP address to the connection.
///         let ip = peer_address.to_string();
///         stream.send_all(ip).await?;
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
    ///
    /// # Notes
    ///
    /// The CPU affinity is **not** set on the returned TCP stream. To set that
    /// use [`TcpStream::set_auto_cpu_affinity`].
    pub async fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        self.fd
            .accept::<SockAddr>()
            .await
            .map(|(fd, addr)| (TcpStream { fd }, addr.into()))
    }

    /// Returns a stream of incoming [`TcpStream`]s.
    ///
    /// Note that unlike [`accept`] this doesn't return the address because it
    /// uses io_uring's multishot accept (making it faster then calling `accept`
    /// in a loop). See the [`TcpListener`] documentation for an example.
    ///
    /// [`accept`]: TcpListener::accept
    ///
    /// # Notes
    ///
    /// The CPU affinity is **not** set on the returned TCP stream. To set that
    /// use [`TcpStream::set_auto_cpu_affinity`].
    pub fn incoming(&mut self) -> Incoming<'_> {
        Incoming(self.fd.multishot_accept())
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
        f(SockRef::from(&self.fd))
    }
}

/// The [`AsyncIterator`] behind [`TcpListener::incoming`].
#[derive(Debug)]
#[must_use = "AsyncIterators do nothing unless polled"]
pub struct Incoming<'a>(a10::net::MultishotAccept<'a>);

impl<'a> AsyncIterator for Incoming<'a> {
    type Item = io::Result<TcpStream>;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll_next(ctx)
            .map_ok(|fd| TcpStream { fd })
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
