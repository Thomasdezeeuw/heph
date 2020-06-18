use std::convert::TryFrom;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::{fmt, io};

use log::debug;
use mio::net::TcpListener;
use mio::Interest;

use crate::actor::messages::Terminate;
use crate::actor::{self, Actor, AddActorError, NewActor, Spawnable};
use crate::net::TcpStream;
use crate::rt::{ActorOptions, ProcessId, RuntimeAccess, Signal};
use crate::supervisor::Supervisor;

/// A intermediate structure that implements [`NewActor`], creating
/// [`tcp::Server`].
///
/// See [`tcp::Server::setup`] to create this and [`tcp::Server`] for examples.
///
/// [`tcp::Server`]: Server
/// [`tcp::Server::setup`]: Server::setup
#[derive(Debug)]
pub struct ServerSetup<S, NA> {
    /// All fields are in an `Arc` to allow `ServerSetup` to cheaply be cloned
    /// and still be `Send` and `Sync` for use in the setup function of
    /// `Runtime`.
    inner: Arc<ServerSetupInner<S, NA>>,
}

#[derive(Debug)]
struct ServerSetupInner<S, NA> {
    /// The underlying TCP listener, backed by Mio.
    ///
    /// NOTE: not registered with `mio::Poll`.
    listener: TcpListener,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to spawn the actor.
    options: ActorOptions,
}

impl<S, NA, K> NewActor for ServerSetup<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
    K: RuntimeAccess + Spawnable<S, NA>,
{
    type Message = ServerMessage;
    type Argument = ();
    type Actor = Server<S, NA, K>;
    type Error = io::Error;
    type Context = K;

    fn new(
        &mut self,
        mut ctx: actor::Context<Self::Message, K>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let this = &*self.inner;
        // FIXME: `try_clone` is removed in Mio v0.7-alpha.1, use `SO_REUSEPORT`
        // instead, see https://github.com/tokio-rs/mio/pull/1068.
        //let mut listener = this.listener.try_clone()?;

        // FIXME: this is an ugly hack to clone the listener anyway.
        use std::os::unix::io::{AsRawFd, FromRawFd};
        let raw_fd = this.listener.as_raw_fd();
        let net_listener = unsafe { std::net::TcpListener::from_raw_fd(raw_fd) };
        let listener = net_listener.try_clone();
        std::mem::forget(net_listener); // This is `this.listener`.
        let mut listener = listener.map(TcpListener::from_std)?;

        let token = ctx.pid().into();
        ctx.kind()
            .register(&mut listener, token, Interest::READABLE)?;

        Ok(Server {
            ctx,
            listener,
            supervisor: this.supervisor.clone(),
            new_actor: this.new_actor.clone(),
            options: this.options.clone(),
        })
    }
}

impl<S, NA> Clone for ServerSetup<S, NA> {
    fn clone(&self) -> ServerSetup<S, NA> {
        ServerSetup {
            inner: self.inner.clone(),
        }
    }
}

/// An actor that starts a new actor for each accepted TCP connection.
///
/// This actor can start as a thread-local or thread-safe actor. When using the
/// thread-local variant one actor runs per worker thread which spawns
/// thread-local actors to handle the [`TcpStream`]s. See the first example
/// below on how to run this `tcp::Server` as a thread-local actor.
///
/// This actor can also run as thread-safe actor in which case it also spawns
/// thread-safe actors. Note however that using thread-*local* version is
/// recommended. The third example below shows how to run the `tcp::Server` as
/// thread-safe actor.
///
/// # Graceful shutdown
///
/// Graceful shutdown is done by sending it a [`Terminate`] message, see below
/// for an example. The TCP server can also handle (shutdown) process signals,
/// see example 2: my_ip (in the examples directory of the source code) for an
/// example of that.
///
/// # Examples
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection, using the server as a thread-local actor.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::{self, context, NewActor};
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, TcpStream};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::rt::options::Priority;
/// use heph::{rt, ActorOptions, Runtime, RuntimeRef};
///
/// fn main() -> Result<(), rt::Error<io::Error>> {
///     // Create and start the Heph runtime.
///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
/// }
///
/// /// In this setup function we'll spawn the TCP server.
/// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our TCP server. We'll use the default actor options.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let server = tcp::Server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///
///     // We advice to give the TCP server a low priority to prioritise
///     // handling of ongoing requests over accepting new requests possibly
///     // overloading the system.
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let mut actor_ref =
///     runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///     # actor_ref <<= Terminate;
///
///     Ok(())
/// }
///
/// /// Our supervisor for the TCP server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<S, NA> Supervisor<tcp::ServerSetup<S, NA>> for ServerSupervisor
/// where
///     // Trait bounds needed by `tcp::ServerSetup`.
///     S: Supervisor<NA> + Clone + 'static,
///     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = context::ThreadLocal> + Clone + 'static,
/// {
///     fn decide(&mut self, err: tcp::ServerError<!>) -> SupervisorStrategy<()> {
///         use tcp::ServerError::*;
///         match err {
///             // When we hit an error accepting a connection we'll drop the old
///             // server and create a new one.
///             Accept(err) => {
///                 error!("error accepting new connection: {}", err);
///                 SupervisorStrategy::Restart(())
///             }
///             // Async function never return an error creating a new actor.
///             NewActor(_) => unreachable!(),
///         }
///     }
///
///     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
///         // If we can't create a new server we'll stop.
///         error!("error restarting the TCP server: {}", err);
///         SupervisorStrategy::Stop
///     }
///
///     fn second_restart_error(&mut self, _: io::Error) {
///         // We don't restart a second time, so this will never be called.
///         unreachable!();
///     }
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// The actor responsible for a single TCP stream.
/// async fn conn_actor(_ctx: actor::Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.write_all(b"Hello World").await
/// }
/// ```
///
/// The following example shows how the actor can gracefully be shutdown by
/// sending it a [`Terminate`] message.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// # use heph::actor::context;
/// use heph::actor::messages::Terminate;
/// use heph::{actor, NewActor};
/// use heph::log::error;
/// use heph::net::tcp::{self, TcpStream};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::rt::options::Priority;
/// use heph::{rt, ActorOptions, Runtime, RuntimeRef};
///
/// fn main() -> Result<(), rt::Error<io::Error>> {
///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
///     // This uses the same supervisors as in the previous example, not shown
///     // here.
///
///     // Adding the TCP server is the same as in the example above.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let server = tcp::Server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     let mut server_ref = runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///
///     // Because the server is just another actor we can send it messages.
///     // Here we'll send it a terminate message so it will gracefully
///     // shutdown.
///     server_ref <<= Terminate;
///
///     Ok(())
/// }
///
/// # /// # Our supervisor for the TCP server.
/// # #[derive(Copy, Clone, Debug)]
/// # struct ServerSupervisor;
/// #
/// # impl<S, NA> Supervisor<tcp::ServerSetup<S, NA>> for ServerSupervisor
/// # where
/// #     S: Supervisor<NA> + Clone + 'static,
/// #     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = context::ThreadLocal> + Clone + 'static,
/// # {
/// #     fn decide(&mut self, err: tcp::ServerError<!>) -> SupervisorStrategy<()> {
/// #         use tcp::ServerError::*;
/// #         match err {
/// #             Accept(err) => {
/// #                 error!("error accepting new connection: {}", err);
/// #                 SupervisorStrategy::Restart(())
/// #             }
/// #             NewActor(_) => unreachable!(),
/// #         }
/// #     }
/// #
/// #     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
/// #         error!("error restarting the TCP server: {}", err);
/// #         SupervisorStrategy::Stop
/// #     }
/// #
/// #     fn second_restart_error(&mut self, _: io::Error) {
/// #         // We don't restart a second time, so this will never be called.
/// #         unreachable!();
/// #     }
/// # }
/// #
/// # /// # `conn_actor`'s supervisor.
/// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
/// #     error!("error handling connection: {}", err);
/// #     SupervisorStrategy::Stop
/// # }
/// #
/// /// The actor responsible for a single TCP stream.
/// async fn conn_actor(_ctx: actor::Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.write_all(b"Hello World").await
/// }
/// ```
///
/// This example is similar to the first example, but runs the `tcp::Server`
/// actor as thread-safe actor. *It's recommended to run the server as
/// thread-local actor!* This is just an example show its possible.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::{self, NewActor};
/// use heph::actor::context::ThreadSafe;
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, TcpStream};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::rt::options::Priority;
/// use heph::{rt, ActorOptions, Runtime};
///
/// fn main() -> Result<(), rt::Error<io::Error>> {
///     let mut runtime = Runtime::new().map_err(rt::Error::map_type)?;
///
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our TCP server. We'll use the default actor options.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let server = tcp::Server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let mut actor_ref =
///     runtime.try_spawn(ServerSupervisor, server, (), options)?;
///     # actor_ref <<= Terminate;
///
///     runtime.start().map_err(rt::Error::map_type)
/// }
///
/// /// Our supervisor for the TCP server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<S, NA> Supervisor<tcp::ServerSetup<S, NA>> for ServerSupervisor
/// where
///     // Trait bounds needed by `tcp::ServerSetup` using a thread-safe actor.
///     S: Supervisor<NA> + Send + Sync + Clone + 'static,
///     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = ThreadSafe> + Send + Sync + Clone + 'static,
///     NA::Actor: Send + Sync + 'static,
///     NA::Message: Send,
/// {
///     fn decide(&mut self, err: tcp::ServerError<!>) -> SupervisorStrategy<()> {
///         use tcp::ServerError::*;
///         match err {
///             // When we hit an error accepting a connection we'll drop the old
///             // server and create a new one.
///             Accept(err) => {
///                 error!("error accepting new connection: {}", err);
///                 SupervisorStrategy::Restart(())
///             }
///             // Async function never return an error creating a new actor.
///             NewActor(_) => unreachable!(),
///         }
///     }
///
///     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
///         // If we can't create a new server we'll stop.
///         error!("error restarting the TCP server: {}", err);
///         SupervisorStrategy::Stop
///     }
///
///     fn second_restart_error(&mut self, _: io::Error) {
///         // We don't restart a second time, so this will never be called.
///         unreachable!();
///     }
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {}", err);
///     SupervisorStrategy::Stop
/// }
///
/// /// The actor responsible for a single TCP stream.
/// async fn conn_actor(_ctx: actor::Context<!, ThreadSafe>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.write_all(b"Hello World").await
/// }
#[derive(Debug)]
pub struct Server<S, NA, K> {
    /// Actor context in which this actor is running.
    ctx: actor::Context<ServerMessage, K>,
    /// The underlying TCP listener, backed by Mio.
    listener: TcpListener,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// `NewActor` used to create an actor for each connection.
    new_actor: NA,
    /// Options used to spawn the actor.
    options: ActorOptions,
}

impl<S, NA, K> Server<S, NA, K>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
{
    /// Create a new [`ServerSetup`].
    ///
    /// Arguments:
    /// * `address`: the address to listen on.
    /// * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    /// * `new_actor`: the [`NewActor`] implementation to start each actor,
    ///   and
    /// * `options`: the actor options used to spawn the new actors.
    pub fn setup(
        address: SocketAddr,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<ServerSetup<S, NA>> {
        TcpListener::bind(address).map(|listener| ServerSetup {
            inner: Arc::new(ServerSetupInner {
                listener,
                supervisor,
                new_actor,
                options,
            }),
        })
    }
}

impl<S, NA, K> Actor for Server<S, NA, K>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
    K: RuntimeAccess + Spawnable<S, NA>,
{
    type Error = ServerError<NA::Error>;

    fn try_poll(
        self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // This is safe because only the `RuntimeRef`, `TcpListener` and
        // the `MailBox` are mutably borrowed and all are `Unpin`.
        let &mut Server {
            ref listener,
            ref mut ctx,
            ref supervisor,
            ref new_actor,
            ref options,
        } = unsafe { self.get_unchecked_mut() };

        // See if we need to shutdown.
        if ctx.try_receive_next().is_some() {
            debug!("TCP server received shutdown message, stopping");
            return Poll::Ready(Ok(()));
        }

        // Next start accepting streams.
        loop {
            let (mut stream, addr) = match listener.accept() {
                Ok(ok) => ok,
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue, // Try again.
                Err(err) => return Poll::Ready(Err(ServerError::Accept(err))),
            };
            debug!("tcp::Server accepted connection: remote_address={}", addr);

            let setup_actor = move |pid: ProcessId, ctx: &mut K| {
                ctx.register(
                    &mut stream,
                    pid.into(),
                    Interest::READABLE | Interest::WRITABLE,
                )?;
                Ok((TcpStream { socket: stream }, addr))
            };
            let res = ctx.kind().spawn(
                supervisor.clone(),
                new_actor.clone(),
                setup_actor,
                options.clone(),
            );
            if let Err(err) = res {
                return Poll::Ready(Err(err.into()));
            }
        }
    }
}

/// The message type used by [`tcp::Server`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`Signal`]`>` for the message, allowing for graceful shutdown.
///
/// [`tcp::Server`]: Server
#[derive(Debug)]
pub struct ServerMessage {
    // Allow for future expansion.
    inner: (),
}

impl From<Terminate> for ServerMessage {
    fn from(_: Terminate) -> ServerMessage {
        ServerMessage { inner: () }
    }
}

impl TryFrom<Signal> for ServerMessage {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(ServerMessage { inner: () }),
        }
    }
}

/// Error returned by the [`tcp::Server`] actor.
///
/// [`tcp::Server`]: Server
#[derive(Debug)]
pub enum ServerError<E> {
    /// Error accepting TCP stream.
    Accept(io::Error),
    /// Error creating a new actor to handle the TCP stream.
    NewActor(E),
}

// Not part of the public API.
#[doc(hidden)]
impl<E> From<AddActorError<E, io::Error>> for ServerError<E> {
    fn from(err: AddActorError<E, io::Error>) -> ServerError<E> {
        match err {
            AddActorError::NewActor(err) => ServerError::NewActor(err),
            AddActorError::ArgFn(err) => ServerError::Accept(err),
        }
    }
}

impl<E: fmt::Display> fmt::Display for ServerError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ServerError::*;
        match self {
            Accept(ref err) => write!(f, "error accepting TCP stream: {}", err),
            NewActor(ref err) => write!(f, "error creating new actor: {}", err),
        }
    }
}
