//! Module with [`UnixDatagram`].

use std::marker::PhantomData;
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd, IntoRawFd};
use std::{fmt, io};

use a10::{AsyncFd, Extract};
use socket2::{Domain, SockRef, Type};

use crate::access::Access;
use crate::io::{Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::net::uds::UnixAddr;
use crate::net::{
    Recv, RecvFrom, RecvFromVectored, RecvVectored, Send, SendTo, SendToVectored, SendVectored,
};
use crate::wakers::NoRing;

#[doc(no_inline)]
pub use crate::net::{Connected, Unconnected};

/// A Unix datagram socket.
///
/// To create a socket [`UnixDatagram::bind`] or [`UnixDatagram::unbound`] can
/// be used. The created socket will be in unconnected mode. A socket can be in
/// one of two modes:
///
/// - [`Unconnected`] mode allows sending and receiving packets to and from all
///   sources.
/// - [`Connected`] mode only allows sending and receiving packets from/to a
///   single source.
///
/// An unconnected socket can be [`connect`ed] to a specific address if needed,
/// changing the mode to [`Connected`] in the process. The remote address of an
/// already connected socket can be changed to a different address using the
/// same method.
///
/// Both unconnected and connected sockets have three main operations send,
/// receive and peek, all these methods return a [`Future`].
///
/// [`connect`ed]: UnixDatagram::connect
/// [`Future`]: std::future::Future
pub struct UnixDatagram<M = Unconnected> {
    fd: AsyncFd,
    /// The mode in which the socket is in, this determines what methods are
    /// available.
    mode: PhantomData<M>,
}

impl UnixDatagram {
    /// Creates a Unix datagram socket bound to `address`.
    pub async fn bind<RT>(rt: &RT, address: UnixAddr) -> io::Result<UnixDatagram<Unconnected>>
    where
        RT: Access,
    {
        let socket = UnixDatagram::unbound(rt).await?;
        socket.with_ref(|socket| socket.bind(&address.inner))?;
        Ok(socket)
    }

    /// Creates a Unix Datagram socket which is not bound to any address.
    pub async fn unbound<RT>(rt: &RT) -> io::Result<UnixDatagram<Unconnected>>
    where
        RT: Access,
    {
        let fd = NoRing(a10::net::socket(
            rt.submission_queue(),
            Domain::UNIX.into(),
            Type::DGRAM.cloexec().into(),
            0,
            0,
        ))
        .await?;
        UnixDatagram::new(rt, fd)
    }

    /// Creates an unnamed pair of connected sockets.
    pub fn pair<RT>(rt: &RT) -> io::Result<(UnixDatagram<Connected>, UnixDatagram<Connected>)>
    where
        RT: Access,
    {
        let (s1, s2) = socket2::Socket::pair(Domain::UNIX, Type::DGRAM.cloexec(), None)?;
        let s1 = UnixDatagram::new(rt, unsafe {
            // SAFETY: the call to `pair` above ensures the file descriptors are
            // valid.
            AsyncFd::from_raw_fd(s1.into_raw_fd(), rt.submission_queue())
        })?;
        let s2 = UnixDatagram::new(rt, unsafe {
            // SAFETY: Same as above.
            AsyncFd::from_raw_fd(s2.into_raw_fd(), rt.submission_queue())
        })?;
        Ok((s1, s2))
    }

    fn new<RT, M>(rt: &RT, fd: AsyncFd) -> io::Result<UnixDatagram<M>>
    where
        RT: Access,
    {
        let socket = UnixDatagram {
            fd,
            mode: PhantomData,
        };

        #[cfg(target_os = "linux")]
        socket.with_ref(|socket| {
            if let Some(cpu) = rt.cpu() {
                if let Err(err) = socket.set_cpu_affinity(cpu) {
                    log::warn!("failed to set CPU affinity on UnixDatagram: {err}");
                }
            }
            Ok(())
        })?;

        Ok(socket)
    }
}

impl<M> UnixDatagram<M> {
    /// Connects the socket by setting the default destination and limiting
    /// packets that are received and send to the `remote` address.
    pub async fn connect(self, remote: UnixAddr) -> io::Result<UnixDatagram<Connected>> {
        NoRing(self.fd.connect(remote)).await?;
        Ok(UnixDatagram {
            fd: self.fd,
            mode: PhantomData,
        })
    }

    /// Converts a [`std::os::unix::net::UnixDatagram`] to a
    /// [`heph_rt::net::UnixDatagram`].
    ///
    /// [`heph_rt::net::UnixDatagram`]: UnixDatagram
    ///
    /// # Notes
    ///
    /// It's up to the caller to ensure that the socket's mode is correctly set
    /// to [`Connected`] or [`Unconnected`].
    pub fn from_std<RT>(rt: &RT, socket: std::os::unix::net::UnixDatagram) -> UnixDatagram<M>
    where
        RT: Access,
    {
        UnixDatagram {
            fd: AsyncFd::new(socket.into(), rt.submission_queue()),
            mode: PhantomData,
        }
    }

    /// Creates a new independently owned `UnixDatagram` that shares the same
    /// underlying file descriptor as the existing `UnixDatagram`.
    pub fn try_clone(&self) -> io::Result<UnixDatagram<M>> {
        Ok(UnixDatagram {
            fd: self.fd.try_clone()?,
            mode: PhantomData,
        })
    }

    /// Returns the socket address of the remote peer of this socket.
    pub fn peer_addr(&self) -> io::Result<UnixAddr> {
        self.with_ref(|socket| socket.peer_addr().map(|a| UnixAddr { inner: a }))
    }

    /// Returns the socket address of the local half of this socket.
    pub fn local_addr(&self) -> io::Result<UnixAddr> {
        self.with_ref(|socket| socket.local_addr().map(|a| UnixAddr { inner: a }))
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

impl UnixDatagram<Unconnected> {
    /// Receives data from the unconnceted socket.
    pub async fn recv_from<B: BufMut>(&self, buf: B) -> io::Result<(B, UnixAddr)> {
        RecvFrom(self.fd.recvfrom(BufWrapper(buf), 0)).await
    }

    /// Receives data from the unconnected socket, using vectored I/O.
    pub async fn recv_from_vectored<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<(B, UnixAddr)> {
        RecvFromVectored(self.fd.recvfrom_vectored(BufWrapper(bufs), 0)).await
    }

    /// Receives data from the unconnected socket, without removing it from the
    /// input queue.
    pub async fn peek_from<B: BufMut>(&self, buf: B) -> io::Result<(B, UnixAddr)> {
        RecvFrom(self.fd.recvfrom(BufWrapper(buf), libc::MSG_PEEK)).await
    }

    /// Receives data from the unconnected socket, without removing it from the
    /// input queue, using vectored I/O.
    pub async fn peek_from_vectored<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<(B, UnixAddr)> {
        RecvFromVectored(self.fd.recvfrom_vectored(BufWrapper(bufs), libc::MSG_PEEK)).await
    }

    /// Send the bytes in `buf` to `address`.
    pub async fn send_to<B: Buf>(&self, buf: B, address: UnixAddr) -> io::Result<(B, usize)> {
        SendTo(self.fd.sendto(BufWrapper(buf), address, 0).extract()).await
    }

    /// Send the bytes in `bufs` to `address`, using vectored I/O.
    pub async fn send_to_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
        address: UnixAddr,
    ) -> io::Result<(B, usize)> {
        SendToVectored(
            self.fd
                .sendto_vectored(BufWrapper(bufs), address, 0)
                .extract(),
        )
        .await
    }
}

impl UnixDatagram<Connected> {
    /// Receive bytes from the connected socket.
    pub async fn recv<B: BufMut>(&self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), 0)).await
    }

    /// Receives data from the connected socket, using vectored I/O.
    pub async fn recv_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), 0)).await
    }

    /// Receive bytes from the connected socket, without removing it from the
    /// input queue, writing them into `buf`.
    pub async fn peek<B: BufMut>(&self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), libc::MSG_PEEK)).await
    }

    /// Receive bytes from the connected socket, without removing it from the
    /// input queue, using vectored I/O.
    pub async fn peek_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), libc::MSG_PEEK)).await
    }

    /// Sends data on the socket to the connected socket.
    pub async fn send<B: Buf>(&self, buf: B) -> io::Result<(B, usize)> {
        Send(self.fd.send(BufWrapper(buf), 0).extract()).await
    }

    /// Sends data on the socket to the connected socket, using vectored I/O.
    pub async fn send_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        SendVectored(self.fd.send_vectored(BufWrapper(bufs), 0).extract()).await
    }
}

impl<M> AsFd for UnixDatagram<M> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<M> fmt::Debug for UnixDatagram<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
