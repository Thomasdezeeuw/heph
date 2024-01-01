//! Module with [`TcpListener`] and related types.

use std::async_iter::AsyncIterator;
use std::net::SocketAddr;
use std::os::fd::{AsFd, BorrowedFd};
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use a10::AsyncFd;
use socket2::{Domain, Protocol, SockRef, Socket, Type};

use crate::access::Access;
use crate::net::{convert_address, SockAddr, TcpStream};
use crate::wakers::NoRing;

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
/// use heph::actor;
/// use heph_rt::ThreadLocal;
/// use heph_rt::net::TcpListener;
/// use log::info;
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///
///     // Accept a connection.
///     let (stream, peer_address) = listener.accept().await?;
///     info!("accepted connection from: {peer_address}");
///
///     // Next we write the IP address to the connection.
///     let ip = peer_address.to_string();
///     stream.send_all(ip).await?;
///     Ok(())
/// }
/// # _ = actor; // Silence unused item warning.
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
/// use heph::actor;
/// use heph_rt::ThreadLocal;
/// use heph_rt::net::TcpListener;
/// use heph_rt::util::next;
/// use log::info;
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let listener = TcpListener::bind(ctx.runtime_ref(), address).await?;
///     let mut incoming = listener.incoming();
///     loop {
///         let stream = match next(&mut incoming).await {
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
/// # _ = actor; // Silence unused warning.
/// ```
pub struct TcpListener {
    fd: AsyncFd,
}

impl TcpListener {
    /// Creates a new `TcpListener` which will be bound to the specified
    /// `address`.
    pub async fn bind<RT>(rt: &RT, address: SocketAddr) -> io::Result<TcpListener>
    where
        RT: Access,
    {
        TcpListener::bind_setup(rt, address, |_| Ok(())).await
    }

    pub(crate) async fn bind_setup<RT, F>(
        rt: &RT,
        address: SocketAddr,
        setup: F,
    ) -> io::Result<TcpListener>
    where
        RT: Access,
        F: FnOnce(&Socket) -> io::Result<()>,
    {
        let fd = NoRing(a10::net::socket(
            rt.submission_queue(),
            Domain::for_address(address).into(),
            Type::STREAM.cloexec().into(),
            Protocol::TCP.into(),
            0,
        ))
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
            socket.listen(libc::SOMAXCONN)?;

            Ok(())
        })?;

        Ok(socket)
    }

    /// Converts a [`std::net::TcpListener`] to a [`heph_rt::net::TcpListener`].
    ///
    /// [`heph_rt::net::TcpListener`]: TcpListener
    pub fn from_std<RT>(rt: &RT, listener: std::net::TcpListener) -> TcpListener
    where
        RT: Access,
    {
        TcpListener {
            fd: AsyncFd::new(listener.into(), rt.submission_queue()),
        }
    }

    /// Creates a new independently owned `TcpListener` that shares the same
    /// underlying file descriptor as the existing `TcpListener`.
    pub fn try_clone(&self) -> io::Result<TcpListener> {
        Ok(TcpListener {
            fd: self.fd.try_clone()?,
        })
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.with_ref(|socket| socket.local_addr().and_then(convert_address))
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.with_ref(|socket| socket.set_ttl(ttl))
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&self) -> io::Result<u32> {
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
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        NoRing(self.fd.accept::<SockAddr>())
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
    #[allow(clippy::doc_markdown)] // For "io_uring".
    pub const fn incoming(&self) -> Incoming<'_> {
        Incoming(self.fd.multishot_accept())
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
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

impl AsFd for TcpListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
