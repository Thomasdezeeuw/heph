//! TCP related types.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::pin::Pin;
use std::task::{LocalWaker, Poll};

use futures_io::{AsyncRead, AsyncWrite, Initializer};
use log::debug;

use mio_st::net::{TcpListener as MioTcpListener, TcpStream as MioTcpStream};
use mio_st::poll::PollOption;

use crate::actor::messages::Terminate;
use crate::actor::{Actor, Context, NewActor};
use crate::mailbox::MailBox;
use crate::net::{interrupted, would_block};
use crate::supervisor::Supervisor;
use crate::system::{ActorOptions, ActorSystemRef, AddActorError};
use crate::util::Shared;

/// A TCP listener.
///
/// This listener will accept TCP connections and for each incoming connection
/// create an actor, via the [`NewActor`] trait.
///
/// # Graceful shutdown
///
/// Graceful shutdown of the TCP listener is done by sending it a [`Terminate`]
/// message, see below for an example.
///
/// # Examples
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
/// # #![allow(unreachable_code)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::Context;
/// use heph::log::error;
/// use heph::net::{TcpListener, TcpListenerError, TcpStream};
/// use heph::supervisor::SupervisorStrategy;
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// # // Don't actually want to run the actor system.
/// # return;
///
/// // Create and run the actor system.
/// ActorSystem::new()
///     .with_setup(setup)
///     .run()
///     .unwrap();
///
/// /// In this setup function we'll add the TcpListener to the actor system.
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // Create our TCP listener. We'll use the default actor options.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let listener = TcpListener::new(conn_supervisor, new_actor, ActorOptions::default());
///
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     system_ref.try_spawn(listener_supervisor, listener, address, ActorOptions {
///         // We advice to give the TCP listener a low priority to prioritise
///         // handling of ongoing requests over accepting new requests possibly
///         // overloading the system.
///         priority: Priority::LOW,
///         .. Default::default()
///     })?;
///
///     Ok(())
/// }
///
/// /// Supervisor for the TCP listener.
/// fn listener_supervisor(err: TcpListenerError<!>) -> SupervisorStrategy<(SocketAddr)> {
///     error!("error accepting connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// The actor responsible for a single TCP stream.
/// async fn conn_actor(_ctx: Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     await!(stream.write_all(b"Hello World"))
/// }
/// ```
///
/// The next example shows how the TCP listener can gracefully be shutdown by
/// sending it a [`Terminate`] message.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::Context;
/// use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::{TcpListener, TcpListenerError, TcpStream};
/// use heph::supervisor::SupervisorStrategy;
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// // Create and run the actor system.
/// ActorSystem::new()
///     .with_setup(setup)
///     .run()
///     .unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // Adding the TCP listener is the same as in the example above.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let listener = TcpListener::new(conn_supervisor, new_actor, ActorOptions::default());
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let mut listener_ref = system_ref.try_spawn(listener_supervisor, listener, address, ActorOptions {
///         priority: Priority::LOW,
///         .. Default::default()
///     })?;
///
///     // Because the listener is just another actor we can send it messages.
///     // Here we'll send it a terminate message so it will gracefully
///     // shutdown.
///     listener_ref <<= Terminate;
///
///     Ok(())
/// }
///
/// /// Supervisor for the TCP listener.
/// fn listener_supervisor(err: TcpListenerError<!>) -> SupervisorStrategy<(SocketAddr)> {
///     error!("error accepting connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// The actor responsible for a single TCP stream.
/// async fn conn_actor(_ctx: Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     await!(stream.write_all(b"Hello World"))
/// }
/// ```
#[derive(Debug)]
pub struct TcpListener<S, NA> {
    /// Reference to the actor system used to add new actors to handle accepted
    /// connections.
    system_ref: ActorSystemRef,
    /// The underlying TCP listener, backed by mio.
    listener: MioTcpListener,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
    /// The inbox of the listener.
    inbox: Shared<MailBox<TcpListenerMessage>>,
}

impl<S, NA> TcpListener<S, NA>
    where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + Clone + Send + 'static,
          NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + Send + 'static,
{
    /// Create a new TCP listener.
    ///
    /// For each accepted connection a new actor will be created by using the
    /// [`NewActor::new`] method, with a `TcpStream` and `SocketAddr` as
    /// argument. The provided `options` will be used in adding the newly
    /// created actor to the actor system. The supervisor `S` will be used as
    /// supervisor.
    ///
    /// Note in the function call we'll not yet bound to the port, this will
    /// happen in `NewTcpListener`'s `NewActor` implementation.
    pub fn new(supervisor: S, new_actor: NA, options: ActorOptions) ->
        impl NewActor<Message = TcpListenerMessage, Argument = SocketAddr, Actor = TcpListener<S, NA>, Error = io::Error>
    {
        NewTcpListener { supervisor, new_actor, options }
    }
}

impl<S, NA> TcpListener<S, NA> {
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

impl<S, NA> Actor for TcpListener<S, NA>
    where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + Clone + 'static,
          NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    type Error = TcpListenerError<NA::Error>;

    fn try_poll(self: Pin<&mut Self>, _waker: &LocalWaker) -> Poll<Result<(), Self::Error>> {
        // This is safe because only the `ActorSystemRef`, `MioTcpListener` and
        // the `MailBox` are mutably borrowed and all are `Unpin`.
        let &mut TcpListener {
            ref mut system_ref,
            ref mut listener,
            ref supervisor,
            ref new_actor,
            ref options,
            ref mut inbox,
        } = unsafe { self.get_unchecked_mut() };

        // Start with receiving any send messages.
        while let Some(msg) = inbox.borrow_mut().receive_next() {
            use self::TcpListenerMessageInner::*;
            match msg.inner {
                Terminate => return Poll::Ready(Ok(())),
            }
        }

        // Next start accepting streams.
        loop {
            let (mut stream, addr) = match listener.accept() {
                Ok(ok) => ok,
                Err(ref err) if would_block(err) => return Poll::Pending,
                Err(ref err) if interrupted(err) => continue, // Try again.
                Err(err) => return Poll::Ready(Err(TcpListenerError::Accept(err))),
            };
            debug!("accepted connection from: {}", addr);

            let res = system_ref.try_spawn_setup(supervisor.clone(), new_actor.clone(), |pid, system_ref| {
                system_ref.poller_register(&mut stream, pid.into(),
                    MioTcpStream::INTERESTS, PollOption::Edge)?;

                // Wrap the raw stream with our wrapper.
                let stream = TcpStream { inner: stream };

                // Return the arguments used to create the actor.
                Ok((stream, addr))
            }, options.clone());

            match res {
                Ok(_) => {}, // Continue.
                Err(err) => return Poll::Ready(Err(err.into())),
            }
        }
    }
}

/// The message type used by [`TcpListener`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` for the
/// message, allowing for graceful shutdown.
#[derive(Debug)]
pub struct TcpListenerMessage {
    inner: TcpListenerMessageInner,
}

impl From<Terminate> for TcpListenerMessage {
    fn from(_msg: Terminate) -> TcpListenerMessage {
        TcpListenerMessage {
            inner: TcpListenerMessageInner::Terminate,
        }
    }
}

/// Internal message type for the TCP listener.
#[derive(Debug)]
enum TcpListenerMessageInner {
    Terminate,
}

/// Error returned by [`TcpListener`].
#[derive(Debug)]
pub enum TcpListenerError<E> {
    /// Error accepting TCP stream.
    Accept(io::Error),
    /// Error creating new actor to handle the TCP stream.
    NewActor(E),
}

impl<E> From<AddActorError<E, io::Error>> for TcpListenerError<E> {
    fn from(err: AddActorError<E, io::Error>) -> TcpListenerError<E> {
        match err {
            AddActorError::NewActor(err) => TcpListenerError::NewActor(err),
            AddActorError::ArgFn(err) => TcpListenerError::Accept(err),
        }
    }
}

impl<E: fmt::Display> fmt::Display for TcpListenerError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::TcpListenerError::*;
        match self {
            Accept(ref err) => write!(f, "error accepting TCP stream: {}", err),
            NewActor(ref err) => write!(f, "error creating new actor: {}", err),
        }
    }
}

/// `NewTcpListener` is an implementation of [`NewActor`] for `TcpListener`.
///
/// It can be created by calling [`TcpListener::new`].
#[derive(Debug, Clone)]
struct NewTcpListener<S, NA> {
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
}

impl<S, NA> NewActor for NewTcpListener<S, NA>
    where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + Clone + Send + 'static,
          NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + Send + 'static,
{
    type Message = TcpListenerMessage;
    type Argument = SocketAddr;
    type Actor = TcpListener<S, NA>;
    type Error = io::Error;

    fn new(&mut self, mut ctx: Context<Self::Message>, address: Self::Argument) -> Result<Self::Actor, Self::Error> {
        let mut system_ref = ctx.system_ref().clone();
        let mut listener = MioTcpListener::bind(address)?;
        system_ref.poller_register(&mut listener, ctx.pid().into(), MioTcpListener::INTERESTS, PollOption::Edge)?;

        Ok(TcpListener {
            system_ref,
            listener,
            supervisor: self.supervisor.clone(),
            new_actor: self.new_actor.clone(),
            options: self.options.clone(),
            inbox: ctx.inbox,
        })
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
    pub fn connect<M>(ctx: &mut Context<M>, address: SocketAddr) -> io::Result<TcpStream> {
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
