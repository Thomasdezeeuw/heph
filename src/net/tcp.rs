//! TCP related types.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::task::{LocalWaker, Poll};

use futures_io::{AsyncRead, AsyncWrite, Initializer};
use log::debug;

use mio_st::net::{TcpListener as MioTcpListener, TcpStream as MioTcpStream};
use mio_st::poll::{PollOption, Poller};

use crate::actor::{ActorContext, Actor, NewActor};
use crate::initiator::Initiator;
use crate::net::{interrupted, would_block};
use crate::process::ProcessId;
use crate::supervisor::Supervisor;
use crate::system::{ActorOptions, ActorSystemRef};

/// A TCP listener that implements the [`Initiator`] trait.
///
/// This listener will accept TCP connections and for each incoming connection
/// create an actor, via the [`NewActor`] trait.
///
/// [`Initiator`]: ../initiator/trait.Initiator.html
/// [`NewActor`]: ../actor/trait.NewActor.html
///
/// # Example
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::ActorContext;
/// use heph::log::{error, log};
/// use heph::net::{TcpListener, TcpStream};
/// use heph::supervisor::SupervisorStrategy;
/// use heph::system::{ActorOptions, ActorSystem, InitiatorOptions};
///
/// async fn conn_actor(_ctx: ActorContext<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
///     await!(stream.write_all(b"Hello World"))
/// }
///
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// // The address to listen on.
/// let address = "127.0.0.1:7890".parse().unwrap();
///
/// // Create our TCP listener. We'll use the default actor options.
/// let new_actor = conn_actor as fn(_, _, _) -> _;
/// let listener = TcpListener::bind(address, conn_supervisor, new_actor, ActorOptions::default())
///     .expect("unable to bind TCP listener");
///
/// // We create our actor system.
/// ActorSystem::new()
///     // We add our TCP listener, using the default options.
///     .with_initiator(listener, InitiatorOptions::default())
///     # ; // We don't actually want to run this.
/// ```
#[derive(Debug)]
pub struct TcpListener<NA, S> {
    /// The underlying TCP listener, backed by mio.
    listener: MioTcpListener,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
}

impl<NA, S> TcpListener<NA, S>
    where NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + Send + 'static,
          S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + Clone + Send + 'static,
{
    /// Bind a new TCP listener to the provided `address`.
    ///
    /// For each accepted connection a new actor will be created by using the
    /// [`NewActor::new`] method with a `TcpStream` and `SocketAddr`. The
    /// provided `options` will be used in adding the newly created actor to the
    /// actor system.
    ///
    /// [`NewActor::new`]: ../actor/trait.NewActor.html#tymethod.new
    pub fn bind(address: SocketAddr, supervisor: S, new_actor: NA, options: ActorOptions) -> io::Result<TcpListener<NA, S>> {
        Ok(TcpListener {
            listener: MioTcpListener::bind(address)?,
            supervisor,
            new_actor,
            options,
        })
    }
}

impl<NA, S> TcpListener<NA, S> {
    /// Returns the local socket address of this listener.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.listener.set_ttl(ttl)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.listener.ttl()
    }
}

impl<NA, S> Initiator for TcpListener<NA, S>
    where NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + Send + 'static,
          S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + Clone + Send + 'static,
{
    #[doc(hidden)]
    fn clone_threaded(&self) -> io::Result<Self> {
        Ok(TcpListener {
            listener: self.listener.try_clone()?,
            options: self.options.clone(),
            new_actor: self.new_actor.clone(),
            supervisor: self.supervisor.clone(),
        })
    }

    #[doc(hidden)]
    fn init(&mut self, poller: &mut Poller, pid: ProcessId) -> io::Result<()> {
        poller.register(&mut self.listener, pid.into(),
            MioTcpListener::INTERESTS, PollOption::Edge)
    }

    #[doc(hidden)]
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()> {
        loop {
            let (mut stream, addr) = match self.listener.accept() {
                Ok(ok) => ok,
                Err(ref err) if would_block(err) => return Ok(()),
                Err(ref err) if interrupted(err) => continue, // Try again.
                Err(err) => return Err(err),
            };
            debug!("accepted connection from: {}", addr);

            let _ = system_ref.add_actor_setup(self.supervisor.clone(), self.new_actor.clone(), |pid, poller| {
                poller.register(&mut stream, pid.into(),
                    MioTcpStream::INTERESTS, PollOption::Edge)?;

                // Wrap the raw stream with our wrapper.
                let stream = TcpStream { inner: stream };

                // Return the arguments used to create the actor.
                Ok((stream, addr))
            }, self.options.clone())?;
        }
    }
}

/// A non-blocking TCP stream between a local socket and a remote socket.
#[derive(Debug)]
pub struct TcpStream {
    /// Underlying TCP connection, backed by mio.
    inner: MioTcpStream,
}

/// A macro to try an I/O function.
// TODO: this is duplicated in the UDP module.
macro_rules! try_io {
    ($op:expr) => {
        loop {
            match $op {
                Ok(ok) => return Poll::Ready(Ok(ok)),
                Err(ref err) if would_block(err) => return Poll::Pending,
                Err(ref err) if interrupted(err) => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    };
}

impl TcpStream {
    /// Create a new TCP stream and issue a non-blocking connect to the
    /// specified `address`.
    pub fn connect<M>(ctx: &mut ActorContext<M>, address: SocketAddr) -> io::Result<TcpStream> {
        let mut stream = MioTcpStream::connect(address)?;
        let pid = ctx.pid();
        ctx.system_ref().poller_register(&mut stream, pid.into(),
            MioTcpStream::INTERESTS, PollOption::Edge)?;
        Ok(TcpStream { inner: stream })
    }

    /// Returns the socket address of the remote peer of this TCP connection.
    pub fn peer_addr(&mut self) -> io::Result<SocketAddr> {
        self.inner.peer_addr()
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.inner.set_ttl(ttl)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.inner.ttl()
    }

    /// Sets the value of the `TCP_NODELAY` option on this socket.
    pub fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.inner.set_nodelay(nodelay)
    }

    /// Gets the value of the `TCP_NODELAY` option on this socket.
    pub fn nodelay(&mut self) -> io::Result<bool> {
        self.inner.nodelay()
    }

    /// Receives data on the socket from the remote address to which it is
    /// connected, without removing that data from the queue. On success,
    /// returns the number of bytes peeked. Successive calls return the same
    /// data.
    pub fn poll_peek(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        try_io!(self.inner.peek(buf))
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified
    /// portions to return immediately with an appropriate value (see the
    /// documentation of [`Shutdown`]).
    ///
    /// [`Shutdown`]: https://doc.rust-lang.org/nightly/std/net/enum.Shutdown.html
    pub fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        self.inner.shutdown(how)
    }

    /// Get the value of the `SO_ERROR` option on this socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing
    /// the field in the process. This can be useful for checking errors between
    /// calls.
    pub fn take_error(&mut self) -> io::Result<Option<io::Error>> {
        self.inner.take_error()
    }
}

impl AsyncRead for TcpStream {
    unsafe fn initializer(&self) -> Initializer {
        Initializer::nop()
    }

    fn poll_read(&mut self, _waker: &LocalWaker, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        try_io!(self.inner.read(buf))
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(&mut self, _waker: &LocalWaker, buf: &[u8]) -> Poll<io::Result<usize>> {
        try_io!(self.inner.write(buf))
    }

    fn poll_flush(&mut self, _waker: &LocalWaker) -> Poll<io::Result<()>> {
        try_io!(self.inner.flush())
    }

    fn poll_close(&mut self, waker: &LocalWaker) -> Poll<io::Result<()>> {
        self.poll_flush(waker)
    }
}
