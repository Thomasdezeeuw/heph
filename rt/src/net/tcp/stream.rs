//! Module with [`TcpStream`] and related types.

use std::io;
use std::net::{Shutdown, SocketAddr};
use std::os::fd::{AsFd, BorrowedFd};

use a10::{AsyncFd, Extract};
use socket2::{Domain, Protocol, SockRef, Type};

use crate::access::Access;
use crate::io::{impl_read, impl_write, Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::net::{
    convert_address, Recv, RecvN, RecvNVectored, RecvVectored, Send, SendAll, SendAllVectored,
    SendVectored, SockAddr,
};
use crate::wakers::NoRing;

/// A non-blocking TCP stream between a local socket and a remote socket.
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
/// use heph_rt::net::TcpStream;
/// use heph_rt::ThreadLocal;
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
///     // Connect to an IP address.
///     let address = "127.0.0.1:12345".parse().unwrap();
///     let stream = TcpStream::connect(ctx.runtime_ref(), address).await?;
///
///     // Send them a nice greeting.
///     stream.send_all("Hello world!").await?;
///     Ok(())
/// }
/// # _ = actor; // Silent dead code warnings.
/// ```
#[derive(Debug)]
pub struct TcpStream {
    pub(in crate::net) fd: AsyncFd,
}

impl TcpStream {
    /// Create a new TCP stream and issues a non-blocking connect to the
    /// specified `address`.
    pub async fn connect<RT>(rt: &RT, address: SocketAddr) -> io::Result<TcpStream>
    where
        RT: Access,
    {
        let fd = NoRing(a10::net::socket(
            rt.submission_queue(),
            Domain::for_address(address).into(),
            Type::STREAM.cloexec().into(),
            Protocol::TCP.into(),
            0,
        ))
        .await?;
        let socket = TcpStream { fd };
        socket.set_auto_cpu_affinity(rt);
        NoRing(socket.fd.connect(SockAddr::from(address))).await?;
        Ok(socket)
    }

    /// Converts a [`std::net::TcpStream`] to a [`heph_rt::net::TcpStream`].
    ///
    /// [`heph_rt::net::TcpStream`]: TcpStream
    pub fn from_std<RT>(rt: &RT, stream: std::net::TcpStream) -> TcpStream
    where
        RT: Access,
    {
        TcpStream {
            fd: AsyncFd::new(stream.into(), rt.submission_queue()),
        }
    }

    /// Creates a new independently owned `TcpStream` that shares the same
    /// underlying file descriptor as the existing `TcpStream`.
    pub fn try_clone(&self) -> io::Result<TcpStream> {
        Ok(TcpStream {
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
    /// This is already called when the `TcpStream` is created using
    /// [`TcpStream::connect`], this is mostly useful when accepting a
    /// connection from [`TcpListener`].
    ///
    /// [`TcpListener`]: crate::net::tcp::TcpListener
    pub fn set_auto_cpu_affinity<RT>(&self, rt: &RT)
    where
        RT: Access,
    {
        #[cfg(target_os = "linux")]
        if let Some(cpu) = rt.cpu() {
            if let Err(err) = self.set_cpu_affinity(cpu) {
                log::warn!("failed to set CPU affinity on TcpStream: {err}");
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

    /// Returns the socket address of the remote peer of this TCP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.with_ref(|socket| socket.peer_addr().and_then(convert_address))
    }

    /// Returns the socket address of the local half of this TCP connection.
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

    /// Sets the value of the `TCP_NODELAY` option on this socket.
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.with_ref(|socket| socket.set_nodelay(nodelay))
    }

    /// Gets the value of the `TCP_NODELAY` option on this socket.
    pub fn nodelay(&self) -> io::Result<bool> {
        self.with_ref(|socket| socket.nodelay())
    }

    /// Returns `true` if `SO_KEEPALIVE` is set.
    pub fn keepalive(&self) -> io::Result<bool> {
        self.with_ref(|socket| socket.keepalive())
    }

    /// Enables or disables `SO_KEEPALIVE`.
    pub fn set_keepalive(&self, enable: bool) -> io::Result<()> {
        self.with_ref(|socket| socket.set_keepalive(enable))
    }

    /// Send the bytes in `buf` to the peer.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `buf`. To ensure that all bytes are written use [`TcpStream::send_all`].
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
    /// use heph_rt::net::TcpStream;
    /// use heph_rt::ThreadLocal;
    ///
    /// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = "127.0.0.1:12345".parse().unwrap();
    ///     let stream = TcpStream::connect(ctx.runtime_ref(), address).await?;
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
    /// use heph_rt::net::TcpStream;
    /// use heph_rt::ThreadLocal;
    ///
    /// async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = "127.0.0.1:12345".parse().unwrap();
    ///     let stream = TcpStream::connect(ctx.runtime_ref(), address).await?;
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
            "called `TcpStream::recv_n` with a buffer smaller then `n`"
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
            "called `TcpStream::recv_n_vectored` with a buffer smaller then `n`"
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

    /* TODO: add `sendfile(2)` wrappers io_uring at the time of writing doesn't support this.
    /// Send the `file` out this stream.
    ///
    /// What kind of files are support depends on the OS and is determined by
    /// the [`FileSend`] trait. All OSs at least support regular files.
    ///
    /// The `offset` is the offset into the `file` from which to start copying.
    /// The `length` is the amount of bytes to copy, or if `None` this send the
    /// entire `file`.
    ///
    /// Users might want to use [`TcpStream::send_file_all`] to ensure all the
    /// specified bytes (between `offset` and `length`) are send.
    pub fn send_file<'a, 'f, F>(
        &'a self,
        file: &'f F,
        offset: usize,
        length: Option<NonZeroUsize>,
    ) -> SendFile<'a, 'f, F>
    where
        F: FileSend,
    {
        SendFile {
            stream: self,
            file,
            offset,
            length,
        }
    }

    /// Same as [`TcpStream::send_all`] but then for [`TcpStream::send_file`].
    ///
    /// Users who want to send the entire file might want to use the
    /// [`TcpStream::send_entire_file`] method.
    pub fn send_file_all<'a, 'f, F>(
        &'a self,
        file: &'f F,
        offset: usize,
        length: Option<NonZeroUsize>,
    ) -> SendFileAll<'a, 'f, F>
    where
        F: FileSend,
    {
        SendFileAll {
            stream: self,
            file,
            start: offset,
            end: length.and_then(|length| NonZeroUsize::new(offset + length.get())),
        }
    }

    /// Convenience method to send the entire `file`.
    ///
    /// See [`TcpStream::send_file`] for more information.
    pub fn send_entire_file<'a, 'f, F>(&'a self, file: &'f F) -> SendFileAll<'a, 'f, F>
    where
        F: FileSend,
    {
        self.send_file_all(file, 0, None)
    }
    */

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

impl_read!(TcpStream, &TcpStream);
impl_write!(TcpStream, &TcpStream);

impl AsFd for TcpStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
