//! Module with [`UnixStream`] and related types.

use std::io;
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd, IntoRawFd};

use a10::{AsyncFd, Extract};
use socket2::{Domain, SockRef, Type};

use crate::access::Access;
use crate::io::{impl_read, impl_write, Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::net::uds::UnixAddr;
use crate::net::{
    Recv, RecvN, RecvNVectored, RecvVectored, Send, SendAll, SendAllVectored, SendVectored,
};
use crate::wakers::NoRing;

/// A non-blocking Unix stream.
///
/// # Examples
///
/// Sending `Hello world!` to a peer.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
///
/// use heph::actor;
/// use heph_rt::ThreadLocal;
/// use heph_rt::net::uds::{UnixStream, UnixAddr};
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
///     let address = UnixAddr::from_pathname("/path/to/my/socket").unwrap();
///     let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;
///     stream.send_all("Hello world!").await?;
///     Ok(())
/// }
/// # _ = actor; // Silent dead code warnings.
/// ```
#[derive(Debug)]
pub struct UnixStream {
    pub(in crate::net) fd: AsyncFd,
}

impl UnixStream {
    /// Create a new Unix stream and issues a non-blocking connect to the
    /// specified `address`.
    pub async fn connect<RT>(rt: &RT, address: UnixAddr) -> io::Result<UnixStream>
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
        let socket = UnixStream::new(rt, fd);
        NoRing(socket.fd.connect(address)).await?;
        Ok(socket)
    }

    /// Creates an unnamed pair of connected sockets.
    pub fn pair<RT>(rt: &RT) -> io::Result<(UnixStream, UnixStream)>
    where
        RT: Access,
    {
        let (s1, s2) = socket2::Socket::pair(Domain::UNIX, Type::STREAM.cloexec(), None)?;
        let s1 = UnixStream::new(rt, unsafe {
            // SAFETY: the call to `pair` above ensures the file descriptors are
            // valid.
            AsyncFd::from_raw_fd(s1.into_raw_fd(), rt.submission_queue())
        });
        let s2 = UnixStream::new(rt, unsafe {
            // SAFETY: Same as above.
            AsyncFd::from_raw_fd(s2.into_raw_fd(), rt.submission_queue())
        });
        Ok((s1, s2))
    }

    fn new<RT>(rt: &RT, fd: AsyncFd) -> UnixStream
    where
        RT: Access,
    {
        let socket = UnixStream { fd };
        socket.set_auto_cpu_affinity(rt);
        socket
    }

    /// Converts a [`std::os::unix::net::UnixStream`] to a
    /// [`heph_rt::net::UnixStream`].
    ///
    /// [`heph_rt::net::UnixStream`]: UnixStream
    pub fn from_std<RT>(rt: &RT, stream: std::os::unix::net::UnixStream) -> UnixStream
    where
        RT: Access,
    {
        UnixStream {
            fd: AsyncFd::new(stream.into(), rt.submission_queue()),
        }
    }

    /// Creates a new independently owned `UnixStream` that shares the same
    /// underlying file descriptor as the existing `UnixStream`.
    pub fn try_clone(&self) -> io::Result<UnixStream> {
        Ok(UnixStream {
            fd: self.fd.try_clone()?,
        })
    }

    /// Automatically set the CPU affinity based on the runtime access `rt`.
    ///
    /// For non-Linux OSs this is a no-op. If `rt` is not local this is also a
    /// no-op.
    ///
    /// # Notes
    ///
    /// This is already called when the `UnixStream` is created using
    /// [`UnixStream::connect`], this is mostly useful when accepting a
    /// connection from [`UnixListener`].
    ///
    /// [`UnixListener`]: crate::net::uds::UnixListener
    pub fn set_auto_cpu_affinity<RT>(&self, rt: &RT)
    where
        RT: Access,
    {
        #[cfg(target_os = "linux")]
        if let Some(cpu) = rt.cpu() {
            if let Err(err) = self.set_cpu_affinity(cpu) {
                log::warn!("failed to set CPU affinity on UnixStream: {err}");
            }
        }
    }

    /// Set the CPU affinity to `cpu`.
    ///
    /// On Linux this uses `SO_INCOMING_CPU`.
    #[cfg(target_os = "linux")]
    pub(crate) fn set_cpu_affinity(&self, cpu: usize) -> io::Result<()> {
        self.with_ref(|socket| socket.set_cpu_affinity(cpu))
    }

    /// Returns the socket address of the remote peer of this Unix connection.
    pub fn peer_addr(&self) -> io::Result<UnixAddr> {
        self.with_ref(|socket| socket.peer_addr().map(|a| UnixAddr { inner: a }))
    }

    /// Returns the socket address of the local half of this Unix connection.
    pub fn local_addr(&self) -> io::Result<UnixAddr> {
        self.with_ref(|socket| socket.local_addr().map(|a| UnixAddr { inner: a }))
    }

    /// Send the bytes in `buf` to the peer.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `buf`. To ensure that all bytes are written use
    /// [`UnixStream::send_all`].
    pub async fn send<B: Buf>(&self, buf: B) -> io::Result<(B, usize)> {
        Send(self.fd.send(BufWrapper(buf), 0).extract()).await
    }

    /// Send the all bytes in `buf` to the peer.
    ///
    /// If this fails to send all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn send_all<B: Buf>(&self, buf: B) -> io::Result<B> {
        SendAll(self.fd.send_all(BufWrapper(buf)).extract()).await
    }

    /// Sends data on the socket to the connected socket, using vectored I/O.
    pub async fn send_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        SendVectored(self.fd.send_vectored(BufWrapper(bufs), 0).extract()).await
    }

    /// Send the all bytes in `bufs` to the peer.
    ///
    /// If this fails to send all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn send_vectored_all<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<B> {
        SendAllVectored(self.fd.send_all_vectored(BufWrapper(bufs)).extract()).await
    }

    /// Receive messages from the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use std::io;
    ///
    /// use heph::actor;
    /// use heph_rt::ThreadLocal;
    /// use heph_rt::net::uds::{UnixStream, UnixAddr};
    ///
    /// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = UnixAddr::from_pathname("/path/to/my/socket").unwrap();
    ///     let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;
    ///
    ///     let buf = Vec::with_capacity(4 * 1024); // 4 KB.
    ///     let buf = stream.recv(buf).await?;
    ///     println!("read {} bytes: {buf:?}", buf.len());
    ///
    ///     Ok(())
    /// }
    /// #
    /// # _ = actor; // Silent dead code warnings.
    /// ```
    pub async fn recv<B: BufMut>(&self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), 0)).await
    }

    /// Receive at least `n` bytes from the stream.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less then `n` bytes could be read.
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use std::io;
    ///
    /// use heph::actor;
    /// use heph_rt::ThreadLocal;
    /// use heph_rt::net::uds::{UnixStream, UnixAddr};
    ///
    /// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = UnixAddr::from_pathname("/path/to/my/socket").unwrap();
    ///     let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;
    ///
    ///     let buf = Vec::with_capacity(4 * 1024); // 4 KB.
    ///     // NOTE: this will return an error if the peer sends less than 1 KB
    ///     // of data before shutting down or closing the connection.
    ///     let n = 1024;
    ///     let buf = stream.recv_n(buf, n).await?;
    ///     println!("read {} bytes: {buf:?}", buf.len());
    ///
    ///     Ok(())
    /// }
    /// #
    /// # _ = actor; // Silent dead code warnings.
    /// ```
    pub async fn recv_n<B: BufMut>(&self, buf: B, n: usize) -> io::Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `UnixStream::recv_n` with a buffer smaller then `n`"
        );
        RecvN(self.fd.recv_n(BufWrapper(buf), n)).await
    }

    /// Receive messages from the stream, using vectored I/O.
    pub async fn recv_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), 0)).await
    }

    /// Receive at least `n` bytes from the stream, using vectored I/O.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less then `n` bytes could be read.
    pub async fn recv_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
        n: usize,
    ) -> io::Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `UnixStream::recv_n_vectored` with a buffer smaller then `n`"
        );
        RecvNVectored(self.fd.recv_n_vectored(BufWrapper(bufs), n)).await
    }

    /// Receive messages from the stream, without removing that data from the
    /// queue.
    pub async fn peek<B: BufMut>(&self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), libc::MSG_PEEK)).await
    }

    /// Receive messages from the stream, without removing it from the input
    /// queue, using vectored I/O.
    pub async fn peek_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), libc::MSG_PEEK)).await
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified
    /// portions to return immediately with an appropriate value (see the
    /// documentation of [`Shutdown`]).
    pub async fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        NoRing(self.fd.shutdown(how)).await
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

impl_read!(UnixStream, &UnixStream);
impl_write!(UnixStream, &UnixStream);

impl AsFd for UnixStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
