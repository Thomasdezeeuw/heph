//! Module with [`TcpStream`] and related types.

// TODO: a number of send/recv methods don't use Mio directly, this is fine on
// Unix but doesn't work on Windows (which we don't support). We need to fix
// that once Mio uses Socket2 and supports all the methods we need, Mio's
// tracking issue: https://github.com/tokio-rs/mio/issues/1381.

use std::future::Future;
use std::io::{self, IoSlice};
use std::net::{Shutdown, SocketAddr};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{self, Poll};

#[cfg(target_os = "linux")]
use log::warn;
use mio::{net, Interest};

use heph::actor;
use socket2::SockRef;

use crate::bytes::{Bytes, BytesVectored, MaybeUninitSlice};
use crate::rt;

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
/// use heph_rt::rt::ThreadLocal;
///
/// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
///     let address = "127.0.0.1:12345".parse().unwrap();
///     let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
///     stream.send_all(b"Hello world!").await
/// }
/// #
/// # drop(actor); // Silent dead code warnings.
/// ```
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
    /// [bound]: heph::actor::Bound
    pub fn connect<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        address: SocketAddr,
    ) -> io::Result<Connect>
    where
        RT: rt::Access,
    {
        let mut socket = net::TcpStream::connect(address)?;
        ctx.runtime()
            .register(&mut socket, Interest::READABLE | Interest::WRITABLE)?;
        Ok(Connect {
            socket: Some(socket),
            #[cfg(target_os = "linux")]
            cpu_affinity: ctx.runtime_ref().cpu(),
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

    /// Set the CPU affinity to `cpu`.
    ///
    /// On Linux this uses `SO_INCOMING_CPU`.
    #[cfg(target_os = "linux")]
    pub(crate) fn set_cpu_affinity(&mut self, cpu: usize) -> io::Result<()> {
        SockRef::from(&self.socket).set_cpu_affinity(cpu)
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
        let socket = SockRef::from(&self.socket);
        socket.keepalive()
    }

    /// Enables or disables `SO_KEEPALIVE`.
    pub fn set_keepalive(&self, enable: bool) -> io::Result<()> {
        let socket = SockRef::from(&self.socket);
        socket.set_keepalive(enable)
    }

    /// Attempt to send bytes in `buf` to the peer.
    ///
    /// If no bytes can currently be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`TcpStream::send`] or [`TcpStream::send_all`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send(&mut self, buf: &[u8]) -> io::Result<usize> {
        SockRef::from(&self.socket).send(buf)
    }

    /// Send the bytes in `buf` to the peer.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `buf`. To ensure that all bytes are written use [`TcpStream::send_all`].
    pub fn send<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Send<'a, 'b> {
        Send { stream: self, buf }
    }

    /// Send the all bytes in `buf` to the peer.
    ///
    /// If this fails to send all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub fn send_all<'a, 'b>(&'a mut self, buf: &'b [u8]) -> SendAll<'a, 'b> {
        SendAll { stream: self, buf }
    }

    /// Attempt to send bytes in `bufs` to the peer.
    ///
    /// If no bytes can currently be send this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`TcpStream::send_vectored`] or [`TcpStream::send_vectored_all`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_send_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        SockRef::from(&self.socket).send_vectored(bufs)
    }

    /// Send the bytes in `bufs` to the peer.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`TcpStream::send_vectored_all`].
    pub fn send_vectored<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> SendVectored<'a, 'b> {
        SendVectored { stream: self, bufs }
    }

    /// Send the all bytes in `bufs` to the peer.
    ///
    /// If this fails to send all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub fn send_vectored_all<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> SendVectoredAll<'a, 'b> {
        SendVectoredAll { stream: self, bufs }
    }

    /// Attempt to receive message(s) from the stream, writing them into `buf`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`TcpStream::recv`] or [`TcpStream::recv_n`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
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
    /// use heph_rt::rt::ThreadLocal;
    ///
    /// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = "127.0.0.1:12345".parse().unwrap();
    ///     let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
    ///
    ///     let mut buf = Vec::with_capacity(4 * 1024); // 4 KB.
    ///     match stream.try_recv(&mut buf) {
    ///         Ok(n) => println!("read {} bytes: {:?}", n, buf),
    ///         Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
    ///             println!("no bytes can't be read at this time");
    ///         },
    ///         Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {
    ///             println!("read got interrupted");
    ///         },
    ///         Err(err) => return Err(err),
    ///     }
    ///
    ///     Ok(())
    /// }
    /// #
    /// # drop(actor); // Silent dead code warnings.
    /// ```
    pub fn try_recv<B>(&mut self, mut buf: B) -> io::Result<usize>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `TcpStream::try_recv with an empty buffer"
        );
        SockRef::from(&self.socket)
            .recv(buf.as_bytes())
            .map(|read| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                read
            })
    }

    /// Receive messages from the stream, writing them into `buf`.
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
    /// use heph_rt::rt::ThreadLocal;
    ///
    /// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = "127.0.0.1:12345".parse().unwrap();
    ///     let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
    ///
    ///     let mut buf = Vec::with_capacity(4 * 1024); // 4 KB.
    ///     let n = stream.recv(&mut buf).await?;
    ///     println!("read {} bytes: {:?}", n, buf);
    ///
    ///     Ok(())
    /// }
    /// #
    /// # drop(actor); // Silent dead code warnings.
    /// ```
    pub fn recv<'a, B>(&'a mut self, buf: B) -> Recv<'a, B>
    where
        B: Bytes,
    {
        Recv { stream: self, buf }
    }

    /// Receive at least `n` bytes from the stream, writing them into `buf`.
    ///
    /// This returns a [`Future`] that receives at least `n` bytes from a
    /// `TcpStream` and writes them into buffer `B`, or returns
    /// [`io::ErrorKind::UnexpectedEof`] if less then `n` bytes could be read.
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
    /// use heph_rt::rt::ThreadLocal;
    ///
    /// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
    ///     let address = "127.0.0.1:12345".parse().unwrap();
    ///     let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
    ///
    ///     let mut buf = Vec::with_capacity(4 * 1024); // 4 KB.
    ///     // NOTE: this will return an error if the peer sends less than 1 KB
    ///     // of data before shutting down or closing the connection.
    ///     let n = 1024;
    ///     stream.recv_n(&mut buf, n).await?;
    ///     println!("read {} bytes: {:?}", n, buf);
    ///
    ///     Ok(())
    /// }
    /// #
    /// # drop(actor); // Silent dead code warnings.
    /// ```
    pub fn recv_n<'a, B>(&'a mut self, buf: B, n: usize) -> RecvN<'a, B>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `TcpStream::recv_n` with a buffer smaller then `n`"
        );
        RecvN {
            stream: self,
            buf,
            left: n,
        }
    }

    /// Attempt to receive message(s) from the stream, writing them into `bufs`.
    ///
    /// If no bytes can currently be received this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`TcpStream::recv_vectored`] or [`TcpStream::recv_n_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_recv_vectored<B>(&mut self, mut bufs: B) -> io::Result<usize>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `TcpStream::try_recv_vectored` with empty buffers"
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

    /// Receive messages from the stream, writing them into `bufs`.
    pub fn recv_vectored<B>(&mut self, bufs: B) -> RecvVectored<'_, B>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `TcpStream::recv_vectored` with empty buffers"
        );
        RecvVectored { stream: self, bufs }
    }

    /// Receive at least `n` bytes from the stream, writing them into `bufs`.
    pub fn recv_n_vectored<B>(&mut self, bufs: B, n: usize) -> RecvNVectored<'_, B>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.spare_capacity() >= n,
            "called `TcpStream::recv_n_vectored` with a buffer smaller then `n`"
        );
        RecvNVectored {
            stream: self,
            bufs,
            left: n,
        }
    }

    /// Attempt to receive messages from the stream, writing them into `buf`,
    /// without removing that data from the queue. On success, returns the
    /// number of bytes peeked.
    pub fn try_peek<B>(&mut self, mut buf: B) -> io::Result<usize>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `TcpStream::try_peek with an empty buffer"
        );
        SockRef::from(&self.socket)
            .peek(buf.as_bytes())
            .map(|read| {
                // Safety: just read the bytes.
                unsafe { buf.update_length(read) }
                read
            })
    }

    /// Receive messages from the stream, writing them into `buf`, without
    /// removing that data from the queue. On success, returns the number of
    /// bytes peeked.
    pub fn peek<'a, B>(&'a mut self, buf: B) -> Peek<'a, B>
    where
        B: Bytes,
    {
        Peek { stream: self, buf }
    }

    /// Attempt to receive messages from the stream using vectored I/O, writing
    /// them into `bufs`, without removing that data from the queue. On success,
    /// returns the number of bytes peeked.
    pub fn try_peek_vectored<B>(&mut self, mut bufs: B) -> io::Result<usize>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `TcpStream::try_peek_vectored` with empty buffers"
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

    /// Receive messages from the stream using vectored I/O, writing them into
    /// `bufs`, without removing that data from the queue. On success, returns
    /// the number of bytes peeked.
    pub fn peek_vectored<B>(&mut self, bufs: B) -> PeekVectored<'_, B>
    where
        B: BytesVectored,
    {
        PeekVectored { stream: self, bufs }
    }

    /// Attempt to make a `sendfile(2)` system call.
    ///
    /// See [`TcpStream::send_file`] for more information.
    pub fn try_send_file<F>(
        &mut self,
        file: &F,
        offset: usize,
        length: Option<NonZeroUsize>,
    ) -> io::Result<usize>
    where
        F: FileSend,
    {
        SockRef::from(&self.socket).sendfile(file, offset, length)
    }

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
        &'a mut self,
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
        &'a mut self,
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
    pub fn send_entire_file<'a, 'f, F>(&'a mut self, file: &'f F) -> SendFileAll<'a, 'f, F>
    where
        F: FileSend,
    {
        self.send_file_all(file, 0, None)
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
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Connect {
    socket: Option<net::TcpStream>,
    #[cfg(target_os = "linux")]
    cpu_affinity: Option<usize>,
}

impl Future for Connect {
    type Output = io::Result<TcpStream>;

    #[track_caller]
    fn poll(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        // This relates directly Mio and `kqueue(2)` and `epoll(2)`. To do a
        // non-blocking TCP connect properly we need to a couple of things.
        //
        // 1. Setup a socket and call `connect(2)`. Mio does this for us.
        //    However it doesn't mean the socket is connected, as we can't
        //    determine that without blocking.
        // 2. To determine if a socket is connected we need to wait for a
        //    `kqueue(2)`/`epoll(2)` event (we get scheduled once we do). But
        //    that doesn't tell us whether or not the socket is connected. To
        //    determine if the socket is connected we need to use `getpeername`
        //    (`TcpStream::peer_addr`). But before checking if we're connected
        //    we need to check for a connection error, by checking `SO_ERROR`
        //    (`TcpStream::take_error`) to not lose that information.
        //    However if we get an event (and thus get scheduled) and
        //    `getpeername` fails with `ENOTCONN` it doesn't actually mean the
        //    socket will never connect properly. So we loop (by returned
        //    `Poll::Pending`) until either `SO_ERROR` is set or the socket is
        //    connected.
        //
        // Sources:
        // * https://cr.yp.to/docs/connect.html
        // * https://stackoverflow.com/questions/17769964/linux-sockets-non-blocking-connect
        match self.socket.take() {
            Some(socket) => {
                // If we hit an error while connecting return that error.
                if let Ok(Some(err)) | Err(err) = socket.take_error() {
                    return Poll::Ready(Err(err));
                }

                // If we can get a peer address it means the stream is
                // connected.
                match socket.peer_addr() {
                    Ok(..) => {
                        #[allow(unused_mut)]
                        let mut stream = TcpStream { socket };
                        #[cfg(target_os = "linux")]
                        if let Some(cpu) = self.cpu_affinity {
                            if let Err(err) = stream.set_cpu_affinity(cpu) {
                                warn!("failed to set CPU affinity on TcpStream: {}", err);
                            }
                        }
                        Poll::Ready(Ok(stream))
                    }
                    // `NotConnected` (`ENOTCONN`) means the socket not yet
                    // connected, but still working on it. `ECONNREFUSED` will
                    // be reported if it fails.
                    Err(err)
                        if err.kind() == io::ErrorKind::NotConnected
                            || err.raw_os_error() == Some(libc::EINPROGRESS) =>
                    {
                        // Socket is not (yet) connected but haven't hit an
                        // error either. So we return `Pending` and wait for
                        // another event.
                        self.socket = Some(socket);
                        Poll::Pending
                    }
                    Err(err) => Poll::Ready(Err(err)),
                }
            }
            None => panic!("polled `tcp::stream::Connect` after completion"),
        }
    }
}

/// The [`Future`] behind [`TcpStream::send`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Send<'a, 'b> {
    stream: &'a mut TcpStream,
    buf: &'b [u8],
}

impl<'a, 'b> Future for Send<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Send { stream, buf } = Pin::into_inner(self);
        try_io!(stream.try_send(*buf))
    }
}

/// The [`Future`] behind [`TcpStream::send_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendAll<'a, 'b> {
    stream: &'a mut TcpStream,
    buf: &'b [u8],
}

impl<'a, 'b> Future for SendAll<'a, 'b> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let SendAll { stream, buf } = Pin::into_inner(self);
        loop {
            match stream.try_send(*buf) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Ok(n) if buf.len() <= n => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *buf = &buf[n..];
                    // Try to send some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// The [`Future`] behind [`TcpStream::send_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendVectored<'a, 'b> {
    stream: &'a mut TcpStream,
    bufs: &'b mut [IoSlice<'b>],
}

impl<'a, 'b> Future for SendVectored<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let SendVectored { stream, bufs } = Pin::into_inner(self);
        try_io!(stream.try_send_vectored(*bufs))
    }
}

/// The [`Future`] behind [`TcpStream::send_vectored_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendVectoredAll<'a, 'b> {
    stream: &'a mut TcpStream,
    bufs: &'b mut [IoSlice<'b>],
}

impl<'a, 'b> Future for SendVectoredAll<'a, 'b> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let SendVectoredAll { stream, bufs } = Pin::into_inner(self);
        while !bufs.is_empty() {
            match stream.try_send_vectored(*bufs) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Ok(n) => IoSlice::advance_slices(bufs, n),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// The [`Future`] behind [`TcpStream::recv`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Recv<'b, B> {
    stream: &'b mut TcpStream,
    buf: B,
}

impl<'b, B> Future for Recv<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Recv { stream, buf } = Pin::into_inner(self);
        try_io!(stream.try_recv(&mut *buf))
    }
}

/// The [`Future`] behind [`TcpStream::peek`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Peek<'b, B> {
    stream: &'b mut TcpStream,
    buf: B,
}

impl<'b, B> Future for Peek<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Peek { stream, buf } = Pin::into_inner(self);
        try_io!(stream.try_peek(&mut *buf))
    }
}

/// The [`Future`] behind [`TcpStream::recv_n`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvN<'b, B> {
    stream: &'b mut TcpStream,
    buf: B,
    left: usize,
}

impl<'b, B> Future for RecvN<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvN { stream, buf, left } = Pin::into_inner(self);
        loop {
            match stream.try_recv(&mut *buf) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Ok(n) if n >= *left => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *left -= n;
                    // Try to read some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// The [`Future`] behind [`TcpStream::recv_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvVectored<'b, B> {
    stream: &'b mut TcpStream,
    bufs: B,
}

impl<'b, B> Future for RecvVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvVectored { stream, bufs } = Pin::into_inner(self);
        try_io!(stream.try_recv_vectored(&mut *bufs))
    }
}

/// The [`Future`] behind [`TcpStream::recv_n_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvNVectored<'b, B> {
    stream: &'b mut TcpStream,
    bufs: B,
    left: usize,
}

impl<'b, B> Future for RecvNVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvNVectored { stream, bufs, left } = Pin::into_inner(self);
        loop {
            match stream.try_recv_vectored(&mut *bufs) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Ok(n) if n >= *left => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *left -= n;
                    // Try to read some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// The [`Future`] behind [`TcpStream::peek_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PeekVectored<'b, B> {
    stream: &'b mut TcpStream,
    bufs: B,
}

impl<'b, B> Future for PeekVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekVectored { stream, bufs } = Pin::into_inner(self);
        try_io!(stream.try_peek_vectored(&mut *bufs))
    }
}

/// The [`Future`] behind [`TcpStream::send_file`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendFile<'a, 'f, F> {
    stream: &'a mut TcpStream,
    file: &'f F,
    offset: usize,
    length: Option<NonZeroUsize>,
}

impl<'a, 'f, F> Future for SendFile<'a, 'f, F>
where
    F: FileSend,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        #[rustfmt::skip]
        let SendFile { stream, file, offset, length } = Pin::into_inner(self);
        try_io!(stream.try_send_file(*file, *offset, *length))
    }
}

/// The [`Future`] behind [`TcpStream::send_file_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendFileAll<'a, 'f, F> {
    stream: &'a mut TcpStream,
    file: &'f F,
    /// Starting and ending offsets into `file`.
    /// If `start >= end` all bytes are send.
    start: usize,
    end: Option<NonZeroUsize>,
}

impl<'a, 'f, F> Future for SendFileAll<'a, 'f, F>
where
    F: FileSend,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        #[rustfmt::skip]
        let SendFileAll { stream, file, start, end } = Pin::into_inner(self);
        loop {
            let length = end.and_then(|end| NonZeroUsize::new(end.get() - *start));
            match stream.try_send_file(*file, *start, length) {
                // If zero bytes are send it means the entire file was send.
                Ok(0) => break Poll::Ready(Ok(())),
                Ok(n) => {
                    *start += n;
                    match end {
                        Some(end) if *start >= end.get() => break Poll::Ready(Ok(())),
                        Some(_) | None => {
                            // If we haven't send all bytes yet, or if we don't
                            // know when to stop (e.g. in case we want to send
                            // the entire file) we must try to send more
                            // bytes because we use edge triggers.
                            continue;
                        }
                    }
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue, // Try again.
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// Trait that determines which types are safe to use in
/// [`TcpStream::try_send_file`], [`TcpStream::send_file`] and
/// [`TcpStream::send_file_all`].
pub trait FileSend: PrivateFileSend {}

use private::PrivateFileSend;

mod private {
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    /// Private version of [`FileSend`].
    ///
    /// [`FileSend`]: super::FileSend
    pub trait PrivateFileSend: AsRawFd {}

    impl super::FileSend for File {}

    impl PrivateFileSend for File {}
}

impl<RT: rt::Access> actor::Bound<RT> for TcpStream {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        ctx.runtime()
            .reregister(&mut self.socket, Interest::READABLE | Interest::WRITABLE)
    }
}
