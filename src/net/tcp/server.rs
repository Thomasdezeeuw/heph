//! Module with [`TcpServer`] and related types.

use std::convert::TryFrom;
use std::net::SocketAddr;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::{fmt, io};

use log::debug;
#[cfg(target_os = "linux")]
use log::warn;
use mio::net::TcpListener;
use mio::Interest;
use socket2::{Domain, Protocol, Socket, Type};

use crate::actor::messages::Terminate;
use crate::actor::{self, Actor, AddActorError, NewActor, PrivateSpawn, Spawn};
use crate::net::TcpStream;
use crate::rt::{self, ActorOptions, PrivateAccess, Signal};
use crate::supervisor::Supervisor;

/// A intermediate structure that implements [`NewActor`], creating
/// [`TcpServer`].
///
/// See [`TcpServer::setup`] to create this and [`TcpServer`] for examples.
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
    socket: Socket,
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

impl<S, NA, K> NewActor for Setup<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
    actor::Context<Message, K>: rt::Access + Spawn<S, NA, K>,
    actor::Context<NA::Message, K>: rt::Access,
{
    type Message = Message;
    type Argument = ();
    type Actor = TcpServer<S, NA, K>;
    type Error = io::Error;
    type Context = K;

    fn new(
        &mut self,
        mut ctx: actor::Context<Self::Message, K>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let this = &*self.inner;
        let socket = new_listener(this.address, 1024)?;
        let mut listener = unsafe { TcpListener::from_raw_fd(socket.into_raw_fd()) };
        let token = ctx.pid().into();
        ctx.register(&mut listener, token, Interest::READABLE)?;
        Ok(TcpServer {
            ctx,
            set_waker: false,
            listener,
            supervisor: this.supervisor.clone(),
            new_actor: this.new_actor.clone(),
            options: this.options.clone(),
        })
    }
}

fn new_listener(address: SocketAddr, backlog: libc::c_int) -> io::Result<Socket> {
    // Create a new non-blocking socket.
    let domain = Domain::for_address(address);
    let ty = Type::STREAM;
    #[cfg(any(target_os = "freebsd", target_os = "linux"))]
    let ty = ty.nonblocking();
    let protocol = Protocol::TCP;
    let socket = Socket::new(domain, ty, Some(protocol))?;
    // For OSs that don't support `SOCK_NONBLOCK`.
    #[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
    socket.set_nonblocking(true)?;

    // Allow the other worker threads and processes to reuse the address and
    // port we're binding to. This allow reload the process without dropping
    // clients.
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?; // TODO: use `SO_REUSEPORT_LB` on FreeBSD.

    // Bind the socket and start listening if required.
    socket.bind(&address.into())?;
    if backlog != 0 {
        socket.listen(backlog)?;
    }

    Ok(socket)
}

impl<S, NA> Clone for Setup<S, NA> {
    fn clone(&self) -> Setup<S, NA> {
        Setup {
            inner: self.inner.clone(),
        }
    }
}

/// An actor that starts a new actor for each accepted TCP connection.
///
/// This actor can start as a thread-local or thread-safe actor. When using the
/// thread-local variant one actor runs per worker thread which spawns
/// thread-local actors to handle the [`TcpStream`]s. See the first example
/// below on how to run this `TcpServer` as a thread-local actor.
///
/// This actor can also run as thread-safe actor in which case it also spawns
/// thread-safe actors. Note however that using thread-*local* version is
/// recommended. The third example below shows how to run the `TcpServer` as
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
/// use heph::actor::{self, context, NewActor};
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{server, TcpServer, TcpStream};
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
///     // Wait for the `TcpStream` to become ready before running the actor.
///     let options = ActorOptions::default().mark_not_ready();
///     let server = TcpServer::setup(address, conn_supervisor, new_actor, options)?;
///
///     // We advice to give the TCP server a low priority to prioritise
///     // handling of ongoing requests over accepting new requests possibly
///     // overloading the system.
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///     # actor_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// /// Our supervisor for the TCP server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<S, NA> Supervisor<server::Setup<S, NA>> for ServerSupervisor
/// where
///     // Trait bounds needed by `server::Setup`.
///     S: Supervisor<NA> + Clone + 'static,
///     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = context::ThreadLocal> + Clone + 'static,
/// {
///     fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
///         use server::Error::*;
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
/// async fn conn_actor(_: actor::Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.send_all(b"Hello World").await
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
/// # use heph::actor::context;
/// use heph::actor::messages::Terminate;
/// use heph::{actor, NewActor};
/// use heph::log::error;
/// use heph::net::{TcpServer, TcpStream};
/// # use heph::net::tcp;
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
///     let server = TcpServer::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     let server_ref = runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///
///     // Because the server is just another actor we can send it messages.
///     // Here we'll send it a terminate message so it will gracefully
///     // shutdown.
///     server_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// # /// # Our supervisor for the TCP server.
/// # #[derive(Copy, Clone, Debug)]
/// # struct ServerSupervisor;
/// #
/// # impl<S, NA> Supervisor<tcp::server::Setup<S, NA>> for ServerSupervisor
/// # where
/// #     S: Supervisor<NA> + Clone + 'static,
/// #     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = context::ThreadLocal> + Clone + 'static,
/// # {
/// #     fn decide(&mut self, err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
/// #         use tcp::server::Error::*;
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
/// async fn conn_actor(_: actor::Context<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.send_all(b"Hello World").await
/// }
/// ```
///
/// This example is similar to the first example, but runs the `TcpServer` actor
/// as thread-safe actor. *It's recommended to run the server as thread-local
/// actor!* This is just an example show its possible.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use heph::actor::{self, NewActor};
/// use heph::actor::context::ThreadSafe;
/// # use heph::actor::messages::Terminate;
/// use heph::log::error;
/// use heph::net::tcp::{server, TcpServer, TcpStream};
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
///     let server = TcpServer::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
///
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime.try_spawn(ServerSupervisor, server, (), options)?;
///     # actor_ref.try_send(Terminate).unwrap();
///
///     runtime.start().map_err(rt::Error::map_type)
/// }
///
/// /// Our supervisor for the TCP server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<S, NA> Supervisor<server::Setup<S, NA>> for ServerSupervisor
/// where
///     // Trait bounds needed by `server::Setup` using a thread-safe actor.
///     S: Supervisor<NA> + Send + Sync + Clone + 'static,
///     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = ThreadSafe> + Send + Sync + Clone + 'static,
///     NA::Actor: Send + Sync + 'static,
///     NA::Message: Send,
/// {
///     fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
///         use server::Error::*;
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
/// async fn conn_actor(_: actor::Context<!, ThreadSafe>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
/// #   drop(address); // Silence dead code warnings.
///     stream.send_all(b"Hello World").await
/// }
#[derive(Debug)]
pub struct TcpServer<S, NA, K> {
    /// Actor context in which this actor is running.
    ctx: actor::Context<Message, K>,
    /// Whether or not we set the waker for the inbox.
    set_waker: bool,
    /// The underlying TCP listener, backed by Mio.
    listener: TcpListener,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// `NewActor` used to create an actor for each connection.
    new_actor: NA,
    /// Options used to spawn the actor.
    options: ActorOptions,
}

impl<S, NA, K> TcpServer<S, NA, K>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
{
    /// Create a new [`Setup`].
    ///
    /// Arguments:
    /// * `address`: the address to listen on.
    /// * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    /// * `new_actor`: the [`NewActor`] implementation to start each actor,
    ///   and
    /// * `options`: the actor options used to spawn the new actors.
    pub fn setup(
        mut address: SocketAddr,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<Setup<S, NA>> {
        // We create a listener which don't actually use. However it gives a
        // nicer user-experience to get an error up-front rather than $n errors
        // later, where $n is the number of cpu cores when spawning a new server
        // on each worker thread.
        //
        // Also note that we use a backlog of `0`, which causes `new_listener`
        // to never call `listen(2)` on the socket.
        new_listener(address, 0).and_then(|socket| {
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
                    socket,
                    address,
                    supervisor,
                    new_actor,
                    options,
                }),
            })
        })
    }
}

impl<S, NA, K> Actor for TcpServer<S, NA, K>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Context = K> + Clone + 'static,
    actor::Context<Message, K>: Spawn<S, NA, K>,
    actor::Context<NA::Message, K>: rt::Access,
{
    type Error = Error<NA::Error>;

    fn try_poll(
        self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // Safety: This is safe because only the `actor::Context` and
        // `set_waker` are mutably borrowed and both are `Unpin`.
        let this = unsafe { Pin::into_inner_unchecked(self) };

        if !this.set_waker {
            // Set the waker of the inbox to ensure we get run when we receive a
            // message.
            this.ctx.register_inbox_waker(ctx.waker());
            this.set_waker = true
        }

        // See if we need to shutdown.
        //
        // We don't return immediately here because we're using `SO_REUSEPORT`,
        // which on most OSes causes each listener (file descriptor) to have
        // there own accept queue. This means that connections in *ours* would
        // be dropped if we would close the file descriptor immediately. So we
        // first accept all pending connections and start actors for them. Note
        // however that there is still a race condition between our last call to
        // `accept` and the time the file descriptor is actually closed,
        // currently we can't avoid this.
        let should_stop = this.ctx.try_receive_next().is_ok();

        loop {
            let (mut stream, addr) = match this.listener.accept() {
                Ok(ok) => ok,
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue, // Try again.
                Err(err) => return Poll::Ready(Err(Error::Accept(err))),
            };
            debug!("TcpServer accepted connection: remote_address={}", addr);

            let setup_actor = move |ctx: &mut actor::Context<NA::Message, K>| {
                ctx.register(
                    &mut stream,
                    ctx.pid().into(),
                    Interest::READABLE | Interest::WRITABLE,
                )?;
                #[allow(unused_mut)]
                let mut stream = TcpStream { socket: stream };
                #[cfg(target_os = "linux")]
                if let Some(cpu) = ctx.cpu() {
                    if let Err(err) = stream.set_cpu_affinity(cpu) {
                        warn!("failed to set CPU affinity on TcpStream: {}", err);
                    }
                }
                Ok((stream, addr))
            };
            let res = this.ctx.try_spawn_setup(
                this.supervisor.clone(),
                this.new_actor.clone(),
                setup_actor,
                this.options.clone(),
            );
            if let Err(err) = res {
                return Poll::Ready(Err(err.into()));
            }
        }

        if should_stop {
            debug!("TCP server received shutdown message, stopping");
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

/// The message type used by [`TcpServer`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`Signal`]`>` for the message, allowing for graceful shutdown.
#[derive(Debug)]
pub struct Message {
    // Allow for future expansion.
    inner: (),
}

impl From<Terminate> for Message {
    fn from(_: Terminate) -> Message {
        Message { inner: () }
    }
}

impl TryFrom<Signal> for Message {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(Message { inner: () }),
        }
    }
}

/// Error returned by the [`TcpServer`] actor.
#[derive(Debug)]
pub enum Error<E> {
    /// Error accepting TCP stream.
    Accept(io::Error),
    /// Error creating a new actor to handle the TCP stream.
    NewActor(E),
}

// Not part of the public API.
#[doc(hidden)]
impl<E> From<AddActorError<E, io::Error>> for Error<E> {
    fn from(err: AddActorError<E, io::Error>) -> Error<E> {
        match err {
            AddActorError::NewActor(err) => Error::NewActor(err),
            AddActorError::ArgFn(err) => Error::Accept(err),
        }
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            Accept(ref err) => write!(f, "error accepting TCP stream: {}", err),
            NewActor(ref err) => write!(f, "error creating new actor: {}", err),
        }
    }
}
