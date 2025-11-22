use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use heph::actor::{self, NewActor, NoMessages};
use heph::supervisor::Supervisor;
use log::{debug, trace};
use socket2::Socket;

use crate::access::{Access, PrivateAccess};
use crate::fd::AsyncFd;
use crate::net::{Domain, Protocol, ServerError, ServerMessage, Type};
use crate::spawn::{ActorOptions, Spawn};
use crate::util::{either, next};

/// TCP server actor.
///
/// The TCP server is a [`NewActor`] that starts a new actor for each accepted
/// TCP connection. This actor can start as a thread-local or thread-safe actor.
///
/// When using the thread-local variant one server should run per worker thread
/// which spawns thread-local actors to handle the connections. See the first
/// example below on how to run this actor as a thread-local actor.
///
/// This actor can also run as a thread-safe actor in which case it also spawns
/// thread-safe actors. Note however that using thread-*local* version is
/// recommended. The third example below shows how to run the actor as
/// thread-safe actor.
///
/// # Graceful shutdown
///
/// Graceful shutdown is done by sending it a [`Terminate`] message, see below
/// for an example. The TCP server can also handle (shutdown) process signals
/// using [`RuntimeRef::receive_signals`]. See "Example 2 my ip" (in the
/// examples directory of the source code) for an example of that.
///
/// [`Terminate`]: heph::messages::Terminate
/// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
///
/// # Examples
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection, using the server as a thread-local actor.
///
/// ```
/// # #![feature(never_type)]
/// use std::io;
///
/// use heph::actor::{self, actor_fn};
/// # use heph::messages::Terminate;
/// use heph::supervisor::SupervisorStrategy;
/// use heph_rt::fd::AsyncFd;
/// use heph_rt::net::{TcpServer, ServerError};
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::spawn::options::Priority;
/// use heph_rt::{Runtime, RuntimeRef, ThreadLocal};
/// use log::error;
///
/// fn main() -> Result<(), heph_rt::Error> {
///     // Create and start the Heph runtime.
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// /// In this setup function we'll spawn the TCP server.
/// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our TCP server.
///     let new_actor = actor_fn(conn_actor);
///     let server = TcpServer::new(address, conn_supervisor, new_actor, ActorOptions::default())?;
///
///     // We advice to give the TCP server a low priority to prioritise
///     // handling of ongoing requests over accepting new requests possibly
///     // overloading the system.
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime_ref.spawn_local(server_supervisor, server, (), options);
///     # actor_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// /// Our supervisor for the TCP server.
/// fn server_supervisor(err: ServerError<!>) -> SupervisorStrategy<()> {
///     match err {
///         // When we hit an error accepting a connection we'll drop the old
///         // server and create a new one.
///         ServerError::Accept(err) => {
///             error!("error accepting new connection: {err}");
///             SupervisorStrategy::Restart(())
///         }
///         // Async function never return an error creating a new actor.
///         ServerError::NewActor(_) => unreachable!(),
///     }
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
///     error!("error handling connection: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// /// The actor responsible for a single connection.
/// async fn conn_actor(_: actor::Context<!, ThreadLocal>, conn: AsyncFd) -> io::Result<()> {
///     conn.send_all("Hello World").await?;
///     Ok(())
/// }
/// ```
///
/// The following example shows how the actor can gracefully be shutdown by
/// sending it a [`Terminate`] message. We'll use the same structure as we did
/// for the previous example, but change the `setup` function.
///
/// ```
/// # #![feature(never_type)]
/// use std::io;
///
/// use heph::actor::{self, actor_fn};
/// use heph::messages::Terminate;
/// # use heph::supervisor::SupervisorStrategy;
/// use heph_rt::fd::AsyncFd;
/// use heph_rt::net::{TcpServer, ServerError};
/// use heph_rt::spawn::options::{ActorOptions, Priority};
/// use heph_rt::RuntimeRef;
/// # use heph_rt::{Runtime, ThreadLocal};
/// use log::error;
///
/// # fn main() -> Result<(), heph_rt::Error> {
/// #     let mut runtime = Runtime::new()?;
/// #     runtime.run_on_workers(setup)?;
/// #     runtime.start()
/// # }
/// #
/// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
///     // This uses the same supervisors as in the previous example, not shown here.
///
///     // Adding the TCP server is the same as in the example above.
///     let new_actor = actor_fn(conn_actor);
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let server = TcpServer::new(address, conn_supervisor, new_actor, ActorOptions::default())?;
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     let server_ref = runtime_ref.spawn_local(server_supervisor, server, (), options);
///
///     // Because the server is just another actor we can send it messages. Here
///     // we'll send it a terminate message so it will gracefully shutdown.
///     server_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// // NOTE: `main`, `server_supervisor`, `conn_supervisor` and `conn_actor` are the same as
/// // in the previous example.
/// #
/// # fn server_supervisor(err: ServerError<!>) -> SupervisorStrategy<()> {
/// #     match err {
/// #         // When we hit an error accepting a connection we'll drop the old
/// #         // server and create a new one.
/// #         ServerError::Accept(err) => {
/// #             error!("error accepting new connection: {err}");
/// #             SupervisorStrategy::Restart(())
/// #         }
/// #         // Async function never return an error creating a new actor.
/// #         ServerError::NewActor(_) => unreachable!(),
/// #     }
/// # }
/// #
/// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
/// #     error!("error handling connection: {err}");
/// #     SupervisorStrategy::Stop
/// # }
/// #
/// # async fn conn_actor(_: actor::Context<!, ThreadLocal>, conn: AsyncFd) -> io::Result<()> {
/// #     conn.send_all("Hello World").await?;
/// #     Ok(())
/// # }
/// ```
///
/// This example is similar to the first example, but runs the TCP server actor
/// as thread-safe actor. *It's recommended to run the server as thread-local
/// actor!* This is just an example show its possible.
///
/// ```
/// # #![feature(never_type)]
/// use std::io;
///
/// use heph::actor::{self, actor_fn};
/// # use heph::messages::Terminate;
/// use heph::supervisor::{SupervisorStrategy};
/// use heph_rt::fd::AsyncFd;
/// use heph_rt::net::{TcpServer, ServerError};
/// use heph_rt::spawn::options::{ActorOptions, Priority};
/// use heph_rt::{self as rt, Runtime, ThreadSafe};
/// use log::error;
///
/// fn main() -> Result<(), rt::Error> {
///     let mut runtime = Runtime::new()?;
///
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our TCP server. We'll use the default actor options.
///     let new_actor = actor_fn(conn_actor);
///     let server = TcpServer::new(address, conn_supervisor, new_actor, ActorOptions::default())
///         .map_err(rt::Error::setup)?;
///
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime.try_spawn(server_supervisor, server, (), options)
///         .map_err(rt::Error::setup)?;
///     # actor_ref.try_send(Terminate).unwrap();
///
///     runtime.start()
/// }
///
/// // NOTE: `server_supervisor`, `conn_supervisor` and `conn_actor` are the same as
/// // in the previous example.
/// #
/// # /// Our supervisor for the TCP server.
/// # fn server_supervisor(err: ServerError<!>) -> SupervisorStrategy<()> {
/// #     match err {
/// #         // When we hit an error accepting a connection we'll drop the old
/// #         // server and create a new one.
/// #         ServerError::Accept(err) => {
/// #             error!("error accepting new connection: {err}");
/// #             SupervisorStrategy::Restart(())
/// #         }
/// #         // Async function never return an error creating a new actor.
/// #         ServerError::NewActor(_) => unreachable!(),
/// #     }
/// # }
/// #
/// # /// `conn_actor`'s supervisor.
/// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
/// #     error!("error handling connection: {err}");
/// #     SupervisorStrategy::Stop
/// # }
/// #
/// # /// The actor responsible for a single connection.
/// # async fn conn_actor(_: actor::Context<!, ThreadSafe>, conn: AsyncFd) -> io::Result<()> {
/// #     conn.send_all("Hello World").await?;
/// #     Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct TcpServer<S, NA> {
    /// All fields are in an `Arc` to allow `Setup` to cheaply be cloned and
    /// still be `Send` and `Sync` for use in the setup function of `Runtime`.
    inner: Arc<Inner<S, NA>>,
}

#[derive(Debug)]
struct Inner<S, NA> {
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

impl<S, NA> TcpServer<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    /// Create a new server.
    ///
    /// Arguments:
    ///  * `address`: the address to listen on.
    ///  * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    ///  * `new_actor`: the [`NewActor`] implementation to start each actor, and
    ///  * `options`: the actor options used to spawn the new actors.
    pub fn new(
        mut address: SocketAddr,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<TcpServer<S, NA>> {
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

            Ok(TcpServer {
                inner: Arc::new(Inner {
                    _socket: socket,
                    address,
                    supervisor,
                    new_actor,
                    options,
                }),
            })
        })
    }

    /// Returns the address the server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.address
    }
}

/// Create a new TCP listener bound to `address`, but **not** listening using
/// blocking I/O.
fn bind_listener(address: SocketAddr) -> io::Result<Socket> {
    let socket = Socket::new(
        socket2::Domain::for_address(address),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
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

impl<S, NA> NewActor for TcpServer<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    type Message = ServerMessage;
    type Argument = ();
    type Actor = impl Future<Output = Result<(), ServerError<NA::Error>>>;
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

impl<S, NA> Clone for TcpServer<S, NA> {
    fn clone(&self) -> TcpServer<S, NA> {
        TcpServer {
            inner: self.inner.clone(),
        }
    }
}

async fn tcp_server<S, NA>(
    mut ctx: actor::Context<ServerMessage, NA::RuntimeAccess>,
    local: SocketAddr,
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> Result<(), ServerError<NA::Error>>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    let listener = a10::net::socket(
        ctx.runtime_ref().sq(),
        Domain::for_address(&local),
        Type::STREAM,
        Some(Protocol::TCP),
    )
    .await
    .map_err(ServerError::Accept)?;
    let l = listener.as_fd().unwrap(); // FIXME.
    let socket = socket2::SockRef::from(&l);
    if let Some(cpu) = ctx.runtime_ref().cpu() {
        if let Err(err) = socket.set_cpu_affinity(cpu) {
            log::warn!("failed to set CPU affinity on TCP server: {err}");
        }
    }
    set_listener_options(&socket).map_err(ServerError::Accept)?;
    socket.bind(&local.into()).map_err(ServerError::Accept)?;
    socket
        .listen(libc::SOMAXCONN)
        .map_err(ServerError::Accept)?;
    trace!(address:% = local; "TCP server listening");

    let mut accept = listener.multishot_accept();
    let mut receive = ctx.receive_next();
    loop {
        match either(next(&mut accept), &mut receive).await {
            Ok(Some(Ok(stream))) => {
                trace!("TCP server accepted connection");
                drop(receive); // Can't double borrow `ctx`.
                if let Some(cpu) = ctx.runtime_ref().cpu() {
                    let s = listener.as_fd().unwrap(); // FIXME.
                    let socket = socket2::SockRef::from(&s);
                    if let Err(err) = socket.set_cpu_affinity(cpu) {
                        log::warn!("failed to set CPU affinity on accepted connection: {err}");
                    }
                }
                _ = ctx
                    .try_spawn(
                        supervisor.clone(),
                        new_actor.clone(),
                        stream,
                        options.clone(),
                    )
                    .map_err(ServerError::NewActor)?;
                receive = ctx.receive_next();
            }
            Ok(Some(Err(err))) => return Err(ServerError::Accept(err)),
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
