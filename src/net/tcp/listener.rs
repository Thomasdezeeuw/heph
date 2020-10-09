//! Module with [`TcpListener`] and related types.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};

use futures_core::future::FusedFuture;
use futures_core::stream::{FusedStream, Stream};
use mio::{net, Interest};

use crate::actor;
use crate::net::TcpStream;
use crate::rt::RuntimeAccess;

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
/// use futures_util::AsyncWriteExt;
/// # use futures_util::AsyncReadExt;
/// use log::error;
///
/// # use heph::net::TcpStream;
/// use heph::actor::Bound;
/// use heph::log::request;
/// use heph::net::TcpListener;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef, SupervisorStrategy};
///
/// fn main() -> Result<(), rt::Error> {
///     heph::log::init();
///
///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:8000".parse().unwrap();
///
///     runtime_ref.spawn_local(supervisor, actor as fn(_, _) -> _, address,
///         ActorOptions::default().mark_ready());
/// #   runtime_ref.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default().mark_ready());
///
///     Ok(())
/// }
/// #
/// # async fn client(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let mut buf = [0; 64];
/// #   let buf = &mut buf[..local_address.len()];
/// #   stream.read_exact(buf).await?;
/// #   assert_eq!(buf, local_address.as_bytes());
/// #   Ok(())
/// # }
///
/// // Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(&mut ctx, address)?;
///
///     // Accept a connection.
///     let (mut stream, peer_address) = listener.accept().await?;
///     request!("accepted connection from: {}", peer_address);
///
///     // Next we need to bind the stream to this actor.
///     // NOTE: if we don't do this the actor will (likely) never be run (again).
///     stream.bind_to(&mut ctx)?;
///
///     // Next we write the IP address to the connection.
///     let ip = peer_address.to_string();
///     stream.write_all(ip.as_bytes()).await
/// }
/// ```
///
/// Accepting multiple [`TcpStream`]s, using [`TcpListener::incoming`].
///
/// ```
/// #![feature(async_closure, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::future::ready;
/// use futures_util::{AsyncWriteExt, TryFutureExt, TryStreamExt};
/// # use futures_util::{AsyncReadExt, StreamExt};
/// use log::error;
///
/// # use heph::net::TcpStream;
/// use heph::actor::Bound;
/// use heph::log::request;
/// use heph::net::TcpListener;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef, SupervisorStrategy};
///
/// fn main() -> Result<(), rt::Error> {
///     heph::log::init();
///
///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let address = "127.0.0.1:8000".parse().unwrap();
///
///     runtime_ref.spawn_local(supervisor, actor as fn(_, _) -> _, address,
///         ActorOptions::default().mark_ready());
/// #   runtime_ref.spawn_local(supervisor, client as fn(_, _) -> _, address, ActorOptions::default().mark_ready());
///
///     Ok(())
/// }
/// #
/// # async fn client(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
/// #   let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
/// #   let local_address = stream.local_addr()?.to_string();
/// #   let mut buf = [0; 64];
/// #   let buf = &mut buf[..local_address.len()];
/// #   stream.read_exact(buf).await?;
/// #   assert_eq!(buf, local_address.as_bytes());
/// #   Ok(())
/// # }
///
/// // Simple supervisor that logs the error and stops the actor.
/// fn supervisor<Arg>(err: io::Error) -> SupervisorStrategy<Arg> {
///     error!("Encountered an error: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
///     // Create a new listener.
///     let mut listener = TcpListener::bind(&mut ctx, address)?;
///     let streams = listener.incoming();
/// #   let streams = streams.take(1);
///
///     streams.try_for_each(|(mut stream, peer_address)| {
///         request!("accepted connection from: {}", peer_address);
///         // Next we need to bind the stream to this actor.
///         // NOTE: if we don't do this the actor will (likely) never be run (again).
///         ready(stream.bind_to(&mut ctx)).and_then(async move |()| {
///             // Next we write the IP address to the connection.
///             let ip = peer_address.to_string();
///             stream.write_all(ip.as_bytes()).await
///         })
///     }).await
/// }
/// ```
#[derive(Debug)]
pub struct TcpListener {
    /// The underlying TCP listener, backed by Mio.
    socket: net::TcpListener,
}

impl TcpListener {
    /// Creates a new `TcpListener` which will be bound to the specified
    /// `address`.
    ///
    /// # Notes
    ///
    /// The listener is also [bound] to the actor that owns the
    /// `actor::Context`, which means the actor will be run every time the
    /// listener has a connection ready to be accepted.
    ///
    /// [bound]: crate::actor::Bound
    pub fn bind<M, K>(
        ctx: &mut actor::Context<M, K>,
        address: SocketAddr,
    ) -> io::Result<TcpListener>
    where
        K: RuntimeAccess,
    {
        let mut socket = net::TcpListener::bind(address)?;
        let pid = ctx.pid();
        ctx.kind()
            .register(&mut socket, pid.into(), Interest::READABLE)?;
        Ok(TcpListener { socket })
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.socket.set_ttl(ttl)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.socket.ttl()
    }

    /// Accepts a new incoming [`TcpStream`].
    ///
    /// If an accepted TCP stream is returned, the remote address of the peer is
    /// returned along with it.
    ///
    /// See the [`TcpListener`] documentation for an example.
    ///
    /// # Notes
    ///
    /// After accepting a stream it needs to be [bound] to an actor to ensure
    /// the actor is run once the stream is ready.
    ///
    /// [bound]: actor::Bound::bind_to
    #[allow(clippy::needless_lifetimes)]
    pub fn accept<'a>(&'a mut self) -> Accept<'a> {
        Accept {
            listener: Some(self),
        }
    }

    /// Returns a stream that iterates over the [`TcpStream`]s being received on
    /// this listener.
    ///
    /// See the [`TcpListener`] documentation for an example.
    ///
    /// # Notes
    ///
    /// After accepting a stream it needs to be [bound] to an actor to ensure
    /// the actor is run once the stream is ready.
    ///
    /// [bound]: actor::Bound::bind_to
    #[allow(clippy::needless_lifetimes)]
    pub fn incoming<'a>(&'a mut self) -> Incoming<'a> {
        Incoming { listener: self }
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

/// The [`Future`] behind [`TcpListener::accept`].
///
/// # Notes
///
/// After accepting a stream it needs to be [bound] to an actor to ensure the
/// actor is run once the stream is ready.
///
/// [bound]: actor::Bound::bind_to
#[derive(Debug)]
pub struct Accept<'a> {
    listener: Option<&'a mut TcpListener>,
}

impl<'a> Future for Accept<'a> {
    type Output = io::Result<(TcpStream, SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        match self.listener {
            Some(ref mut listener) => try_io!(listener.socket.accept())
                .map(|res| {
                    drop(self.listener.take());
                    res
                })
                .map_ok(|(socket, address)| (TcpStream { socket }, address)),
            None => panic!("polled Accept after it return Poll::Ready"),
        }
    }
}

impl<'a> FusedFuture for Accept<'a> {
    fn is_terminated(&self) -> bool {
        self.listener.is_none()
    }
}

/// The [`Stream`] behind [`TcpListener::incoming`].
///
/// # Notes
///
/// After accepting a stream it needs to be [bound] to an actor to ensure the
/// actor is run once the stream is ready.
///
/// [bound]: actor::Bound::bind_to
#[derive(Debug)]
pub struct Incoming<'a> {
    listener: &'a mut TcpListener,
}

impl<'a> Stream for Incoming<'a> {
    type Item = io::Result<(TcpStream, SocketAddr)>;

    fn poll_next(self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        try_io!(self.listener.socket.accept())
            .map_ok(|(socket, address)| (TcpStream { socket }, address))
            .map(Some)
    }
}

impl<'a> FusedStream for Incoming<'a> {
    fn is_terminated(&self) -> bool {
        false
    }
}

impl<K> actor::Bound<K> for TcpListener
where
    K: RuntimeAccess,
{
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, K>) -> io::Result<()> {
        let pid = ctx.pid();
        ctx.kind()
            .reregister(&mut self.socket, pid.into(), Interest::READABLE)
    }
}
