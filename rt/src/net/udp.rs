//! User Datagram Protocol (UDP) related types.
//!
//! See [`UdpSocket`].

use std::marker::PhantomData;
use std::net::SocketAddr;
use std::os::fd::AsFd;
use std::{fmt, io};

use a10::{AsyncFd, Extract};
use socket2::{Domain, Protocol, SockRef, Type};

use crate::io::{Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::net::{
    convert_address, Recv, RecvFrom, RecvFromVectored, RecvVectored, Send, SendTo, SendToVectored,
    SendVectored, SockAddr,
};
use crate::{self as rt};

pub use crate::net::{Connected, Unconnected};

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
/// An unconnected socket can be [`connect`ed] to a specific address if needed,
/// changing the mode to [`Connected`] in the process. The remote address of an
/// already connected socket can be changed to a different address using the
/// same method.
///
/// Both unconnected and connected sockets have three main operations send,
/// receive and peek, all these methods return a [`Future`].
///
/// [`connect`ed]: UdpSocket::connect
/// [`Future`]: std::future::Future
///
/// # Examples
///
/// ```
/// #![feature(never_type)]
///
/// use std::net::SocketAddr;
/// use std::{io, str};
///
/// use log::error;
///
/// use heph::messages::Terminate;
/// use heph::{actor, SupervisorStrategy};
/// use heph_rt::net::UdpSocket;
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::util::either;
/// use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
///
/// fn main() -> Result<(), rt::Error> {
///     std_logger::Config::logfmt().init();
///
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// fn setup(mut runtime: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:7000".parse().unwrap();
///     // Add our server actor.
///     runtime.spawn_local(supervisor, echo_server as fn(_, _) -> _, address, ActorOptions::default());
///     // Add our client actor.
///     runtime.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default());
///     Ok(())
/// }
///
/// /// Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// /// Actor that will bind a UDP socket and waits for incoming packets and
/// /// echos the message to standard out.
/// async fn echo_server(mut ctx: actor::Context<Terminate, ThreadLocal>, local: SocketAddr) -> io::Result<()> {
///     let mut socket = UdpSocket::bind(ctx.runtime_ref(), local).await?;
///     let mut buf = Vec::with_capacity(4096);
///     loop {
///         buf.clear();
///         let receive_msg = ctx.receive_next();
///         let read = socket.recv_from(buf);
///         let address = match either(read, receive_msg).await {
///             // Received a packet.
///             Ok(Ok((b, address))) => {
///                 buf = b; // The buffer will now be filled with data.
///                 address
///             },
///             // Read error.
///             Ok(Err(err)) => return Err(err),
///             // If we receive a terminate message we'll stop the actor.
///             Err(_) => return Ok(()),
///         };
///
///         match str::from_utf8(&buf) {
///             Ok(str) => println!("Got the following message: `{str}`, from {address}"),
///             Err(_) => println!("Got data: {buf:?}, from {address}"),
///         }
/// #       return Ok(());
///     }
/// }
///
/// /// The client that will send a message to the server.
/// async fn client(ctx: actor::Context<!, ThreadLocal>, server_address: SocketAddr) -> io::Result<()> {
///     let local_address = "127.0.0.1:7001".parse().unwrap();
///     let mut socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?
///         .connect(server_address).await?;
///
///     let (msg, n) = socket.send("Hello world").await?;
///     assert_eq!(n, msg.len());
///     Ok(())
/// }
/// ```
pub struct UdpSocket<M = Unconnected> {
    fd: AsyncFd,
    /// The mode in which the socket is in, this determines what methods are
    /// available.
    mode: PhantomData<M>,
}

impl UdpSocket {
    /// Create a UDP socket binding to the `local` address.
    pub async fn bind<RT>(rt: &RT, local: SocketAddr) -> io::Result<UdpSocket<Unconnected>>
    where
        RT: rt::Access,
    {
        let fd = a10::net::socket(
            rt.submission_queue(),
            Domain::for_address(local).into(),
            Type::DGRAM.cloexec().into(),
            Protocol::UDP.into(),
            0,
        )
        .await?;

        let socket = UdpSocket {
            fd,
            mode: PhantomData,
        };

        socket.with_ref(|socket| {
            #[cfg(target_os = "linux")]
            if let Some(cpu) = rt.cpu() {
                if let Err(err) = socket.set_cpu_affinity(cpu) {
                    log::warn!("failed to set CPU affinity on UdpSocket: {err}");
                }
            }

            socket.bind(&local.into())?;

            Ok(())
        })?;

        Ok(socket)
    }
}

impl<M> UdpSocket<M> {
    /// Connects the UDP socket by setting the default destination and limiting
    /// packets that are received, send and peeked to the `remote` address.
    pub async fn connect(self, remote: SocketAddr) -> io::Result<UdpSocket<Connected>> {
        self.fd.connect(SockAddr::from(remote)).await?;
        Ok(UdpSocket {
            fd: self.fd,
            mode: PhantomData,
        })
    }

    /// Returns the sockets peer address.
    pub fn peer_addr(&mut self) -> io::Result<SocketAddr> {
        self.with_ref(|socket| socket.peer_addr().and_then(convert_address))
    }

    /// Returns the sockets local address.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.with_ref(|socket| socket.local_addr().and_then(convert_address))
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&mut self) -> io::Result<Option<io::Error>> {
        self.with_ref(|socket| socket.take_error())
    }

    fn with_ref<F, T>(&self, f: F) -> io::Result<T>
    where
        F: FnOnce(SockRef<'_>) -> io::Result<T>,
    {
        let borrowed = self.fd.as_fd(); // TODO: remove this once we update to socket2 v0.5.
        f(SockRef::from(&borrowed))
    }
}

impl UdpSocket<Unconnected> {
    /// Receives data from the unconnceted socket.
    pub async fn recv_from<B: BufMut>(&mut self, buf: B) -> io::Result<(B, SocketAddr)> {
        RecvFrom::<B, SockAddr>(self.fd.recvfrom(BufWrapper(buf), 0))
            .await
            .map(|(buf, addr)| (buf, addr.into()))
    }

    /// Receives data from the unconnected socket, using vectored I/O.
    pub async fn recv_from_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, SocketAddr)> {
        RecvFromVectored::<B, SockAddr, N>(self.fd.recvfrom_vectored(BufWrapper(bufs), 0))
            .await
            .map(|(bufs, addr)| (bufs, addr.into()))
    }

    /// Receives data from the unconnected socket, without removing it from the
    /// input queue.
    pub async fn peek_from<B: BufMut>(&mut self, buf: B) -> io::Result<(B, SocketAddr)> {
        RecvFrom::<B, SockAddr>(self.fd.recvfrom(BufWrapper(buf), libc::MSG_PEEK))
            .await
            .map(|(buf, addr)| (buf, addr.into()))
    }

    /// Receives data from the unconnected socket, without removing it from the
    /// input queue, using vectored I/O.
    pub async fn peek_from_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, SocketAddr)> {
        RecvFromVectored::<B, SockAddr, N>(
            self.fd.recvfrom_vectored(BufWrapper(bufs), libc::MSG_PEEK),
        )
        .await
        .map(|(buf, addr)| (buf, addr.into()))
    }

    /// Send the bytes in `buf` to `address`.
    pub async fn send_to<'a, B: Buf>(
        &'a mut self,
        buf: B,
        address: SocketAddr,
    ) -> io::Result<(B, usize)> {
        SendTo(
            self.fd
                .sendto(BufWrapper(buf), SockAddr::from(address), 0)
                .extract(),
        )
        .await
    }

    /// Send the bytes in `bufs` to `address`, using vectored I/O.
    pub async fn send_to_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
        address: SocketAddr,
    ) -> io::Result<(B, usize)> {
        SendToVectored(
            self.fd
                .sendto_vectored(BufWrapper(bufs), SockAddr::from(address), 0)
                .extract(),
        )
        .await
    }
}

impl UdpSocket<Connected> {
    /// Receive bytes from the connected socket.
    pub async fn recv<'a, B: BufMut>(&'a mut self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), 0)).await
    }

    /// Receives data from the connected socket, using vectored I/O.
    pub async fn recv_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), 0)).await
    }

    /// Receive bytes from the connected socket, without removing it from the
    /// input queue, writing them into `buf`.
    pub async fn peek<'a, B: BufMut>(&'a mut self, buf: B) -> io::Result<B> {
        Recv(self.fd.recv(BufWrapper(buf), libc::MSG_PEEK)).await
    }

    /// Receive bytes from the connected socket, without removing it from the
    /// input queue, using vectored I/O.
    pub async fn peek_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        RecvVectored(self.fd.recv_vectored(BufWrapper(bufs), libc::MSG_PEEK)).await
    }

    /// Sends data on the socket to the connected socket.
    pub async fn send<'a, B: Buf>(&'a mut self, buf: B) -> io::Result<(B, usize)> {
        Send(self.fd.send(BufWrapper(buf), 0).extract()).await
    }

    /// Sends data on the socket to the connected socket, using vectored I/O.
    pub async fn send_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        SendVectored(self.fd.send_vectored(BufWrapper(bufs), 0).extract()).await
    }
}

impl<M> fmt::Debug for UdpSocket<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}
