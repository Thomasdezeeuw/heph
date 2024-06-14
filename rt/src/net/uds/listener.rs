//! Module with [`UnixListener`] and related types.

use std::async_iter::AsyncIterator;
use std::os::fd::{AsFd, BorrowedFd};
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use a10::AsyncFd;
use socket2::{Domain, SockRef, Type};

use crate::access::Access;
use crate::net::uds::{UnixAddr, UnixStream};
use crate::wakers::NoRing;

/// A Unix socket listener.
///
/// A listener can be created using [`UnixListener::bind`]. After it is created
/// there are two ways to accept incoming [`UnixStream`]s:
///
///  * [`accept`] accepts a single connection, or
///  * [`incoming`] which returns stream of incoming connections.
///
/// [`accept`]: UnixListener::accept
/// [`incoming`]: UnixListener::incoming
///
/// # Examples
///
/// Accepting a single [`UnixStream`], using [`UnixListener::accept`].
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
///
/// use heph::actor;
/// use heph_rt::net::uds::{UnixListener, UnixAddr};
/// use heph_rt::ThreadLocal;
/// use log::info;
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: UnixAddr) -> io::Result<()> {
///     // Create a new listener.
///     let listener = UnixListener::bind(ctx.runtime_ref(), address).await?;
///
///     // Accept a connection.
///     let (stream, peer_address) = listener.accept().await?;
///     info!("accepted connection from: {:?}", peer_address.as_pathname());
///
///     // Next we write the path to the connection.
///     if let Some(path) = peer_address.as_pathname() {
///         stream.send_all(path.display().to_string()).await?;
///     }
///     Ok(())
/// }
/// # _ = actor; // Silent dead code warnings.
/// ```
///
/// Accepting multiple [`UnixStream`]s, using [`UnixListener::incoming`].
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
///
/// use log::info;
///
/// use heph::actor;
/// use heph_rt::net::uds::{UnixListener, UnixAddr};
/// use heph_rt::ThreadLocal;
/// use heph_rt::util::next;
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>, address: UnixAddr) -> io::Result<()> {
///     // Create a new listener.
///     let listener = UnixListener::bind(ctx.runtime_ref(), address).await?;
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
///         info!("accepted connection from: {:?}", peer_address.as_pathname());
///
///         // Next we write the path to the connection.
///         if let Some(path) = peer_address.as_pathname() {
///             stream.send_all(path.display().to_string()).await?;
///         }
///     }
/// }
/// # _ = actor; // Silent dead code warnings.
/// ```
pub struct UnixListener {
    fd: AsyncFd,
}

impl UnixListener {
    /// Creates a Unix socket bound to `address`.
    pub async fn bind<RT>(rt: &RT, address: UnixAddr) -> io::Result<UnixListener>
    where
        RT: Access,
    {
        let fd = NoRing(a10::net::socket(
            rt.submission_queue(),
            Domain::UNIX.into(),
            Type::STREAM.cloexec().into(),
            0,
            0,
        ))
        .await?;

        let socket = UnixListener { fd };

        #[cfg(target_os = "linux")]
        socket.with_ref(|socket| {
            if let Some(cpu) = rt.cpu() {
                if let Err(err) = socket.set_cpu_affinity(cpu) {
                    log::warn!("failed to set CPU affinity on UnixListener: {err}");
                }
            }

            socket.bind(&address.inner)?;
            socket.listen(libc::SOMAXCONN)?;

            Ok(())
        })?;

        Ok(socket)
    }

    /// Converts a [`std::os::unix::net::UnixListener`] to a
    /// [`heph_rt::net::UnixListener`].
    ///
    /// [`heph_rt::net::UnixListener`]: UnixListener
    pub fn from_std<RT>(rt: &RT, listener: std::os::unix::net::UnixListener) -> UnixListener
    where
        RT: Access,
    {
        UnixListener {
            fd: AsyncFd::new(listener.into(), rt.submission_queue()),
        }
    }

    /// Creates a new independently owned `UnixListener` that shares the same
    /// underlying file descriptor as the existing `UnixListener`.
    pub fn try_clone(&self) -> io::Result<UnixListener> {
        Ok(UnixListener {
            fd: self.fd.try_clone()?,
        })
    }

    /// Returns the socket address of the local half of this socket.
    pub fn local_addr(&self) -> io::Result<UnixAddr> {
        self.with_ref(|socket| socket.local_addr().map(|a| UnixAddr { inner: a }))
    }

    /// Accept a new incoming [`UnixStream`].
    ///
    /// Returns the Unix stream and the remote address of the peer. See the
    /// [`UnixListener`] documentation for an example.
    ///
    /// # Notes
    ///
    /// The CPU affinity is **not** set on the returned Unix stream. To set that
    /// use [`UnixStream::set_auto_cpu_affinity`].
    pub async fn accept(&self) -> io::Result<(UnixStream, UnixAddr)> {
        NoRing(self.fd.accept())
            .await
            .map(|(fd, addr)| (UnixStream { fd }, addr))
    }

    /// Returns a stream of incoming [`UnixStream`]s.
    ///
    /// Note that unlike [`accept`] this doesn't return the address because it
    /// uses io_uring's multishot accept (making it faster then calling `accept`
    /// in a loop). See the [`UnixListener`] documentation for an example.
    ///
    /// [`accept`]: UnixListener::accept
    ///
    /// # Notes
    ///
    /// The CPU affinity is **not** set on the returned Unix stream. To set that
    /// use [`UnixStream::set_auto_cpu_affinity`].
    #[allow(clippy::doc_markdown)] // For "io_uring".
    pub fn incoming(&self) -> Incoming<'_> {
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

    fn with_ref<F, T>(&self, f: F) -> io::Result<T>
    where
        F: FnOnce(SockRef<'_>) -> io::Result<T>,
    {
        f(SockRef::from(&self.fd))
    }
}

/// The [`AsyncIterator`] behind [`UnixListener::incoming`].
#[derive(Debug)]
#[must_use = "AsyncIterators do nothing unless polled"]
pub struct Incoming<'a>(a10::net::MultishotAccept<'a>);

impl<'a> AsyncIterator for Incoming<'a> {
    type Item = io::Result<UnixStream>;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }
            .poll_next(ctx)
            .map_ok(|fd| UnixStream { fd })
    }
}

impl AsFd for UnixListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl fmt::Debug for UnixListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
