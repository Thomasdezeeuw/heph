use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::{fmt, io};

use log::debug;
use mio::net::TcpListener;
use mio::Interests;

use crate::actor::messages::Terminate;
use crate::actor::{self, Actor, NewActor};
use crate::inbox::Inbox;
use crate::net::TcpStream;
use crate::supervisor::Supervisor;
use crate::system::ProcessId;
use crate::system::{ActorOptions, ActorSystemRef, AddActorError};

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
    /// `ActorSystem`.
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
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
}

impl<S, NA> NewActor for ServerSetup<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    type Message = ServerMessage;
    type Argument = ();
    type Actor = Server<S, NA>;
    type Error = io::Error;

    fn new(
        &mut self,
        mut ctx: actor::Context<Self::Message>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let mut system_ref = ctx.system_ref().clone();
        let this = &*self.inner;
        let mut listener = this.listener.try_clone()?;
        system_ref.register(&mut listener, ctx.pid().into(), Interests::READABLE)?;

        Ok(Server {
            listener,
            system_ref,
            supervisor: this.supervisor.clone(),
            new_actor: this.new_actor.clone(),
            options: this.options.clone(),
            inbox: ctx.inbox,
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
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::{self, NewActor};
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, TcpStream};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// // Create and run the actor system.
/// ActorSystem::new().with_setup(setup).run()
/// #   .unwrap();
///
/// /// In this setup function we'll add the TcpListener to the actor system.
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
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
///     system_ref.try_spawn(ServerSupervisor, server, (), options)?;
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
///     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !> + Clone + 'static,
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
/// use heph::{actor, NewActor};
/// use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{self, TcpStream};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::system::options::Priority;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef};
///
/// // Create and run the actor system.
/// ActorSystem::new().with_setup(setup).run()
/// #   .unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // This uses the same supervisors as in the previous example, not shown
///     // here.
///
///     // Adding the TCP server is the same as in the example above.
///     let new_actor = conn_actor as fn(_, _, _) -> _;
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let server = tcp::Server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     let mut server_ref = system_ref.try_spawn(ServerSupervisor, server, (), options)?;
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
/// #     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !> + Clone + 'static,
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
#[derive(Debug)]
pub struct Server<S, NA> {
    /// The underlying TCP listener, backed by Mio.
    listener: TcpListener,
    /// Reference to the actor system used to add new actors to handle accepted
    /// connections.
    system_ref: ActorSystemRef,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
    inbox: Inbox<ServerMessage>,
}

impl<S, NA> Server<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    /// Create a new [`ServerSetup`].
    ///
    /// Arguments:
    /// * `address`: the address to listen on.
    /// * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    /// * `new_actor`: the [`NewActor`] implementation to start each actor, and
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

impl<S, NA> Actor for Server<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr)> + Clone + 'static,
{
    type Error = ServerError<NA::Error>;

    fn try_poll(
        self: Pin<&mut Self>,
        _ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // This is safe because only the `ActorSystemRef`, `TcpListener` and
        // the `MailBox` are mutably borrowed and all are `Unpin`.
        let &mut Server {
            ref listener,
            ref mut system_ref,
            ref supervisor,
            ref new_actor,
            ref options,
            ref mut inbox,
        } = unsafe { self.get_unchecked_mut() };

        // See if we need to shutdown.
        if inbox.receive_next().is_some() {
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
                    Interests::READABLE | Interests::WRITABLE,
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

/// The message type used by [`tcp::Server`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` for the message, allowing
/// for graceful shutdown.
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
