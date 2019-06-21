use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use gaea::net::{TcpListener as GaeaTcpListener, TcpStream as GaeaTcpStream};
use gaea::os::RegisterOption;
use log::debug;

use crate::actor::messages::Terminate;
use crate::actor::{self, Actor, NewActor};
use crate::mailbox::MailBox;
use crate::net::TcpStream;
use crate::supervisor::Supervisor;
use crate::system::ProcessId;
use crate::system::{ActorOptions, ActorSystemRef, AddActorError};
use crate::util::Shared;

/// A intermediate structure that implements [`NewActor`], creating
/// [`tcp::Server`].
///
/// See [`setup_server`] to create this and [`tcp::Server`] for examples.
///
/// [`tcp::Server`]: crate::net::tcp::Server
#[derive(Debug)]
pub struct ServerSetup<S, NA> {
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
}

/// Create a new [`ServerSetup`].
///
/// Arguments:
/// * the actor (`new_actor`) to start for each connection,
/// * the `supervisor` to supervise each new actor, and
/// * the actor `options` used to spawn the new actors.
///
/// See [`tcp::Server`] for examples.
///
/// [`tcp::Server`]: crate::net::tcp::Server
pub const fn setup_server<S, NA>(
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> ServerSetup<S, NA> {
    ServerSetup {
        supervisor,
        new_actor,
        options,
    }
}

impl<S, NA> NewActor for ServerSetup<S, NA>
where
    S: Supervisor<<NA::Actor as actor::Actor>::Error, NA::Argument> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    type Message = ServerMessage;
    type Argument = SocketAddr;
    type Actor = Server<S, NA>;
    type Error = io::Error;

    fn new(
        &mut self,
        mut ctx: actor::Context<Self::Message>,
        address: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let mut system_ref = ctx.system_ref().clone();
        let mut listener = GaeaTcpListener::bind(address)?;
        system_ref.register(
            &mut listener,
            ctx.pid().into(),
            GaeaTcpListener::INTERESTS,
            RegisterOption::EDGE,
        )?;

        Ok(Server {
            listener,
            system_ref,
            supervisor: self.supervisor.clone(),
            new_actor: self.new_actor.clone(),
            options: self.options.clone(),
            inbox: ctx.inbox,
        })
    }
}

/// An actor that starts a new actor for each accepted TCP connection.
///
/// See [`tcp::ServerSetup`] for the [`NewActor`] implementation for this actor.
///
/// [`tcp::ServerSetup`]: crate::net::tcp::ServerSetup
///
/// # Graceful shutdown
///
/// Graceful shutdown is done by sending it a [`Terminate`] message, see below
/// for an example.
///
/// # Examples
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection.
///
/// ```
/// #![feature(async_await, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor;
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, ServerError, TcpStream};
/// use heph::supervisor::SupervisorStrategy;
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// // Create and run the actor system.
/// ActorSystem::new().with_setup(setup).run().unwrap();
///
/// /// In this setup function we'll add the TcpListener to the actor system.
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // Create our TCP listener. We'll use the default actor options.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let listener = tcp::setup_server(conn_supervisor, new_actor, ActorOptions::default());
///
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let options = ActorOptions {
///         // We advice to give the TCP listener a low priority to prioritise
///         // handling of ongoing requests over accepting new requests possibly
///         // overloading the system.
///         priority: Priority::LOW,
///         ..Default::default()
///     };
///     # let mut actor_ref =
///     system_ref.try_spawn(listener_supervisor, listener, address, options)?;
///
///     actor_ref <<= Terminate;
///
///     Ok(())
/// }
///
/// /// Supervisor for the TCP listener.
/// fn listener_supervisor(err: ServerError<!>) -> SupervisorStrategy<(SocketAddr)> {
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
/// #![feature(async_await, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor;
/// use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, ServerError, TcpStream};
/// use heph::supervisor::SupervisorStrategy;
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// // Create and run the actor system.
/// ActorSystem::new().with_setup(setup).run().unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // Adding the TCP listener is the same as in the example above.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let listener = tcp::setup_server(conn_supervisor, new_actor, ActorOptions::default());
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let options = ActorOptions {
///         priority: Priority::LOW,
///         ..Default::default()
///     };
///     let mut listener_ref = system_ref.try_spawn(listener_supervisor, listener, address, options)?;
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
/// fn listener_supervisor(err: ServerError<!>) -> SupervisorStrategy<(SocketAddr)> {
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
/// async fn conn_actor(_ctx: actor::Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.write_all(b"Hello World").await
/// }
/// ```
#[derive(Debug)]
pub struct Server<S, NA> {
    /// The underlying TCP listener, backed by Gaea.
    listener: GaeaTcpListener,
    /// Reference to the actor system used to add new actors to handle accepted
    /// connections.
    system_ref: ActorSystemRef,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
    /// The inbox of the listener.
    inbox: Shared<MailBox<ServerMessage>>,
}

impl<S, NA> Actor for Server<S, NA>
where
    S: Supervisor<<NA::Actor as actor::Actor>::Error, NA::Argument> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    type Error = ServerError<NA::Error>;

    fn try_poll(
        self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // This is safe because only the `ActorSystemRef`, `GaeaTcpListener` and
        // the `MailBox` are mutably borrowed and all are `Unpin`.
        let &mut Server {
            ref mut listener,
            ref mut system_ref,
            ref supervisor,
            ref new_actor,
            ref options,
            ref mut inbox,
        } = unsafe { self.get_unchecked_mut() };

        // See if we need to shutdown.
        if inbox.borrow_mut().receive_next().is_some() {
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
            debug!("accepted connection from: {}", addr);

            let setup_actor = move |pid: ProcessId, system_ref: &mut ActorSystemRef| {
                system_ref.register(
                    &mut stream,
                    pid.into(),
                    GaeaTcpStream::INTERESTS,
                    RegisterOption::EDGE,
                )?;
                Ok((TcpStream { socket: stream }, addr))
            };
            let res = system_ref.try_spawn_setup(
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

/// The message type used by [`Actor`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` for the message, allowing
/// for graceful shutdown.
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

/// Error returned by the [`Actor`] actor.
#[derive(Debug)]
pub enum ServerError<E> {
    /// Error accepting TCP stream.
    Accept(io::Error),
    /// Error creating a new actor to handle the TCP stream.
    NewActor(E),
}

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
