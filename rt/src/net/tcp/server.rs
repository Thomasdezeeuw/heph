//! TCP server actor.
//!
//! The TCP server is an actor that starts a new actor for each accepted TCP
//! connection. This actor can start as a thread-local or thread-safe actor.
//! When using the thread-local variant one actor runs per worker thread which
//! spawns thread-local actors to handle the [`TcpStream`]s. See the first
//! example below on how to run this actor as a thread-local actor.
//!
//! This actor can also run as thread-safe actor in which case it also spawns
//! thread-safe actors. Note however that using thread-*local* version is
//! recommended. The third example below shows how to run the actor as
//! thread-safe actor.
//!
//! # Graceful shutdown
//!
//! Graceful shutdown is done by sending it a [`Terminate`] message, see below
//! for an example. The TCP server can also handle (shutdown) process signals,
//! see "Example 2 my ip" (in the examples directory of the source code) for an
//! example of that.
//!
//! # Examples
//!
//! The following example is a TCP server that writes "Hello World" to the
//! connection, using the server as a thread-local actor.
//!
//! ```
//! #![feature(never_type)]
//!
//! use std::io;
//!
//! use heph::actor::{self, actor_fn};
//! # use heph::messages::Terminate;
//! use heph::supervisor::SupervisorStrategy;
//! use heph_rt::net::{tcp, TcpStream};
//! use heph_rt::spawn::ActorOptions;
//! use heph_rt::spawn::options::Priority;
//! use heph_rt::{Runtime, RuntimeRef, ThreadLocal};
//! use log::error;
//!
//! fn main() -> Result<(), heph_rt::Error> {
//!     // Create and start the Heph runtime.
//!     let mut runtime = Runtime::new()?;
//!     runtime.run_on_workers(setup)?;
//!     runtime.start()
//! }
//!
//! /// In this setup function we'll spawn the TCP server.
//! fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
//!     // The address to listen on.
//!     let address = "127.0.0.1:7890".parse().unwrap();
//!     // Create our TCP server.
//!     let new_actor = actor_fn(conn_actor);
//!     let server = tcp::server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
//!
//!     // We advice to give the TCP server a low priority to prioritise
//!     // handling of ongoing requests over accepting new requests possibly
//!     // overloading the system.
//!     let options = ActorOptions::default().with_priority(Priority::LOW);
//!     # let actor_ref =
//!     runtime_ref.spawn_local(server_supervisor, server, (), options);
//!     # actor_ref.try_send(Terminate).unwrap();
//!
//!     Ok(())
//! }
//!
//! /// Our supervisor for the TCP server.
//! fn server_supervisor(err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
//!     match err {
//!         // When we hit an error accepting a connection we'll drop the old
//!         // server and create a new one.
//!         tcp::server::Error::Accept(err) => {
//!             error!("error accepting new connection: {err}");
//!             SupervisorStrategy::Restart(())
//!         }
//!         // Async function never return an error creating a new actor.
//!         tcp::server::Error::NewActor(_) => unreachable!(),
//!     }
//! }
//!
//! /// `conn_actor`'s supervisor.
//! fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
//!     error!("error handling connection: {err}");
//!     SupervisorStrategy::Stop
//! }
//!
//! /// The actor responsible for a single TCP stream.
//! async fn conn_actor(_: actor::Context<!, ThreadLocal>, stream: TcpStream) -> io::Result<()> {
//!     stream.send_all("Hello World").await?;
//!     Ok(())
//! }
//! ```
//!
//! The following example shows how the actor can gracefully be shutdown by
//! sending it a [`Terminate`] message. We'll use the same structure as we did
//! for the previous example, but change the `setup` function.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! use std::io;
//!
//! use heph::actor::{self, actor_fn};
//! use heph::messages::Terminate;
//! # use heph::supervisor::SupervisorStrategy;
//! use heph_rt::net::{tcp, TcpStream};
//! use heph_rt::spawn::options::{ActorOptions, Priority};
//! use heph_rt::RuntimeRef;
//! # use heph_rt::{Runtime, ThreadLocal};
//! use log::error;
//!
//! # fn main() -> Result<(), heph_rt::Error> {
//! #     let mut runtime = Runtime::new()?;
//! #     runtime.run_on_workers(setup)?;
//! #     runtime.start()
//! # }
//! #
//! fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
//!     // This uses the same supervisors as in the previous example, not shown here.
//!
//!     // Adding the TCP server is the same as in the example above.
//!     let new_actor = actor_fn(conn_actor);
//!     let address = "127.0.0.1:7890".parse().unwrap();
//!     let server = tcp::server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
//!     let options = ActorOptions::default().with_priority(Priority::LOW);
//!     let server_ref = runtime_ref.spawn_local(server_supervisor, server, (), options);
//!
//!     // Because the server is just another actor we can send it messages. Here
//!     // we'll send it a terminate message so it will gracefully shutdown.
//!     server_ref.try_send(Terminate).unwrap();
//!
//!     Ok(())
//! }
//!
//! // NOTE: `main`, `server_supervisor`, `conn_supervisor` and `conn_actor` are the same as
//! // in the previous example.
//! #
//! # fn server_supervisor(err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
//! #     match err {
//! #         // When we hit an error accepting a connection we'll drop the old
//! #         // server and create a new one.
//! #         tcp::server::Error::Accept(err) => {
//! #             error!("error accepting new connection: {err}");
//! #             SupervisorStrategy::Restart(())
//! #         }
//! #         // Async function never return an error creating a new actor.
//! #         tcp::server::Error::NewActor(_) => unreachable!(),
//! #     }
//! # }
//! #
//! # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
//! #     error!("error handling connection: {err}");
//! #     SupervisorStrategy::Stop
//! # }
//! #
//! # async fn conn_actor(_: actor::Context<!, ThreadLocal>, stream: TcpStream) -> io::Result<()> {
//! #     stream.send_all("Hello World").await?;
//! #     Ok(())
//! # }
//! ```
//!
//! This example is similar to the first example, but runs the TCP server actor
//! as thread-safe actor. *It's recommended to run the server as thread-local
//! actor!* This is just an example show its possible.
//!
//! ```
//! #![feature(never_type)]
//!
//! use std::io;
//!
//! use heph::actor::{self, actor_fn};
//! # use heph::messages::Terminate;
//! use heph::supervisor::{SupervisorStrategy};
//! use heph_rt::net::{tcp, TcpStream};
//! use heph_rt::spawn::options::{ActorOptions, Priority};
//! use heph_rt::{self as rt, Runtime, ThreadSafe};
//! use log::error;
//!
//! fn main() -> Result<(), rt::Error> {
//!     let mut runtime = Runtime::new()?;
//!
//!     // The address to listen on.
//!     let address = "127.0.0.1:7890".parse().unwrap();
//!     // Create our TCP server. We'll use the default actor options.
//!     let new_actor = actor_fn(conn_actor);
//!     let server = tcp::server::setup(address, conn_supervisor, new_actor, ActorOptions::default())
//!         .map_err(rt::Error::setup)?;
//!
//!     let options = ActorOptions::default().with_priority(Priority::LOW);
//!     # let actor_ref =
//!     runtime.try_spawn(server_supervisor, server, (), options)
//!         .map_err(rt::Error::setup)?;
//!     # actor_ref.try_send(Terminate).unwrap();
//!
//!     runtime.start()
//! }
//! #
//! # /// Our supervisor for the TCP server.
//! # fn server_supervisor(err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
//! #     match err {
//! #         // When we hit an error accepting a connection we'll drop the old
//! #         // server and create a new one.
//! #         tcp::server::Error::Accept(err) => {
//! #             error!("error accepting new connection: {err}");
//! #             SupervisorStrategy::Restart(())
//! #         }
//! #         // Async function never return an error creating a new actor.
//! #         tcp::server::Error::NewActor(_) => unreachable!(),
//! #     }
//! # }
//! #
//! # /// `conn_actor`'s supervisor.
//! # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
//! #     error!("error handling connection: {err}");
//! #     SupervisorStrategy::Stop
//! # }
//! #
//! /// The actor responsible for a single TCP stream.
//! async fn conn_actor(_: actor::Context<!, ThreadSafe>, stream: TcpStream) -> io::Result<()> {
//!     stream.send_all("Hello World").await?;
//!     Ok(())
//! }
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{fmt, io};

use heph::actor::{self, NewActor, NoMessages};
use heph::messages::Terminate;
use heph::supervisor::Supervisor;
use log::{debug, trace};
use socket2::{Domain, Protocol, Socket, Type};

use crate::access::Access;
use crate::net::{TcpListener, TcpStream};
use crate::spawn::{ActorOptions, Spawn};
use crate::util::{either, next};
use crate::Signal;

/// Create a new [server setup].
///
/// Arguments:
///  * `address`: the address to listen on.
///  * `supervisor`: the [`Supervisor`] used to supervise each started actor,
///  * `new_actor`: the [`NewActor`] implementation to start each actor, and
///  * `options`: the actor options used to spawn the new actors.
///
/// See the [module documentation] for examples.
///
/// [server setup]: Setup
/// [module documentation]: crate::net::tcp::server
#[allow(clippy::arc_with_non_send_sync)]
pub fn setup<S, NA>(
    mut address: SocketAddr,
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> io::Result<Setup<S, NA>>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = TcpStream> + Clone + 'static,
{
    // We create a listener which don't actually use. However it gives a
    // nicer user-experience to get an error up-front rather than $n errors
    // later, where $n is the number of cpu cores when spawning a new server
    // on each worker thread.
    bind_listener(address).and_then(|socket| {
        // Using a port of 0 means the OS can select one for us. However
        // we still consistently want to use the same port instead of
        // binding to a number of random ports.
        if address.port() == 0 {
            // NOTE: we just created the socket above so we know it's either
            // IPv4 or IPv6, meaning this `unwrap` never fails.
            address = socket.local_addr()?.as_socket().unwrap();
        }

        Ok(Setup {
            inner: Arc::new(SetupInner {
                _socket: socket,
                address,
                supervisor,
                new_actor,
                options,
            }),
        })
    })
}

/// Create a new TCP listener bound to `address`, but **not** listening using
/// blocking I/O.
fn bind_listener(address: SocketAddr) -> io::Result<Socket> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;

    set_listener_options(&socket)?;

    // Bind the socket and start listening if required.
    socket.bind(&address.into())?;

    Ok(socket)
}

/// Set the desired socket options on `socket`.
fn set_listener_options(socket: &Socket) -> io::Result<()> {
    // Allow the other worker threads and processes to reuse the address and
    // port we're binding to. This allow reload the process without dropping
    // clients.
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    Ok(())
}

/// A intermediate structure that implements [`NewActor`], creating an actor
/// that spawn a new actor for each incoming TCP connection.
///
/// See [`setup`] to create this and the [module documentation] for examples.
///
/// [module documentation]: crate::net::tcp::server
#[derive(Debug)]
pub struct Setup<S, NA> {
    /// All fields are in an `Arc` to allow `Setup` to cheaply be cloned and
    /// still be `Send` and `Sync` for use in the setup function of `Runtime`.
    inner: Arc<SetupInner<S, NA>>,
}

#[derive(Debug)]
struct SetupInner<S, NA> {
    /// Unused socket bound to the `address`, it is just used to return an error
    /// quickly if we can't create the socket or bind to the address.
    _socket: Socket,
    /// Address of the `listener`, used to create new sockets.
    address: SocketAddr,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to spawn the actor.
    options: ActorOptions,
}

impl<S, NA> Setup<S, NA> {
    /// Returns the address the server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.address
    }
}

impl<S, NA> NewActor for Setup<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = TcpStream> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    type Message = Message;
    type Argument = ();
    type Actor = impl Future<Output = Result<(), Error<NA::Error>>>;
    type Error = !;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        (): Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let this = &*self.inner;
        Ok(tcp_server(
            ctx,
            this.address,
            this.supervisor.clone(),
            this.new_actor.clone(),
            this.options.clone(),
        ))
    }
}

impl<S, NA> Clone for Setup<S, NA> {
    fn clone(&self) -> Setup<S, NA> {
        Setup {
            inner: self.inner.clone(),
        }
    }
}

async fn tcp_server<S, NA>(
    mut ctx: actor::Context<Message, NA::RuntimeAccess>,
    local: SocketAddr,
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> Result<(), Error<NA::Error>>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = TcpStream> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    let listener = TcpListener::bind_setup(ctx.runtime_ref(), local, set_listener_options)
        .await
        .map_err(Error::Accept)?;
    trace!(address:% = local; "TCP server listening");

    let mut accept = listener.incoming();
    let mut receive = ctx.receive_next();
    loop {
        match either(next(&mut accept), &mut receive).await {
            Ok(Some(Ok(stream))) => {
                trace!("TCP server accepted connection");
                drop(receive); // Can't double borrow `ctx`.
                stream.set_auto_cpu_affinity(ctx.runtime_ref());
                _ = ctx
                    .try_spawn(
                        supervisor.clone(),
                        new_actor.clone(),
                        stream,
                        options.clone(),
                    )
                    .map_err(Error::NewActor)?;
                receive = ctx.receive_next();
            }
            Ok(Some(Err(err))) => return Err(Error::Accept(err)),
            Ok(None) => {
                debug!("no more connections to accept in TCP server, stopping");
                return Ok(());
            }
            Err(Ok(_)) => {
                debug!("TCP server received shutdown message, stopping");
                return Ok(());
            }
            Err(Err(NoMessages)) => {
                debug!("All actor references to TCP server dropped, stopping");
                return Ok(());
            }
        }
    }
}

/// The message type used by TCP server actor.
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`Signal`]`>` for the message, allowing for graceful shutdown.
#[derive(Debug)]
pub struct Message {
    // Allow for future expansion.
    _inner: (),
}

impl From<Terminate> for Message {
    fn from(_: Terminate) -> Message {
        Message { _inner: () }
    }
}

impl TryFrom<Signal> for Message {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(Message { _inner: () }),
            _ => Err(()),
        }
    }
}

/// Error returned by the TCP server actor.
#[derive(Debug)]
pub enum Error<E> {
    /// Error accepting TCP stream.
    Accept(io::Error),
    /// Error creating a new actor to handle the TCP stream.
    NewActor(E),
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Accept(err) => write!(f, "error accepting TCP stream: {err}"),
            Error::NewActor(err) => write!(f, "error creating new actor: {err}"),
        }
    }
}
