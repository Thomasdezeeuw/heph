use std::future::Future;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::sync::{Arc, Mutex};
use std::{fmt, io, ptr};

use heph::actor::{self, NewActor, NoMessages};
use heph::supervisor::Supervisor;

use crate::access::Access;
#[cfg(any(target_os = "android", target_os = "linux"))]
use crate::access::PrivateAccess;
use crate::fd::AsyncFd;
use crate::net::{Domain, ServerError, ServerMessage, SocketAddress, option};
use crate::spawn::{ActorOptions, Spawn};
use crate::syscall;
use crate::util::{either, next};

/// Server actor.
///
/// The server is a [`NewActor`] that starts a new actor for each accepted
/// connection. This actor can start as a thread-local or thread-safe actor.
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
/// for an example. The server can also handle (shutdown) process signals using
/// [`RuntimeRef::receive_signals`]. See "Example 2 my ip" (in the examples
/// directory of the source code) for an example of that.
///
/// [`Terminate`]: heph::messages::Terminate
/// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
///
/// # TCP & Unix sockets supported
///
/// The listener is created using streams type ([`Type::STREAM`]). This means
/// that it works for TCP & Unix Domain Sockets (UDS), but not for e.g. UDP.
///
/// [`Type::STREAM`]: crate::net::Type::STREAM
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
/// use heph::actor::{self, actor_fn, Actor, NewActor};
/// # use heph::messages::Terminate;
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph_rt::fd::AsyncFd;
/// use heph_rt::net::{Server, ServerError};
/// use heph_rt::spawn::ActorOptions;
/// use heph_rt::spawn::options::Priority;
/// use heph_rt::{Runtime, RuntimeRef, ThreadLocal};
///
/// fn main() -> Result<(), heph_rt::Error> {
///     // Create and start the Heph runtime.
///     let mut runtime = Runtime::new()?;
///     runtime.run_on_workers(setup)?;
///     runtime.start()
/// }
///
/// /// In this setup function we'll spawn the server.
/// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our server.
///     let new_actor = actor_fn(conn_actor);
///     let server = Server::new(address, conn_supervisor, new_actor, ActorOptions::default())?;
///
///     // We advice to give the server a low priority to prioritise handling of ongoing
///     // requests over accepting new requests possibly overloading the system.
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///     # actor_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// /// Our supervisor for the server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<NA> Supervisor<NA> for ServerSupervisor
/// where
///     NA: NewActor<Argument = (), Error = io::Error>,
///     NA::Actor: Actor<Error = ServerError<!>>,
/// {
///     fn decide(&mut self, err: ServerError<!>) -> SupervisorStrategy<()> {
///         match err {
///             // When we hit an error accepting a connection we'll drop the old
///             // listener and create a new one.
///             ServerError::Accept(err) => {
///                 log::error!("error accepting new connection: {err}");
///                 SupervisorStrategy::Restart(())
///             }
///             // Async function never return an error creating a new actor.
///             ServerError::NewActor(err) => err,
///         }
///     }
///
///     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
///         log::error!("failed to restart listener, trying again: {err}");
///         SupervisorStrategy::Restart(())
///     }
///
///     fn second_restart_error(&mut self, err: io::Error) {
///         log::error!("failed to restart listener a second time, stopping it: {err}");
///     }
/// }
///
/// /// `conn_actor`'s supervisor.
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
///     log::error!("error handling connection: {err}");
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
/// use heph_rt::net::{Server, ServerError};
/// use heph_rt::spawn::options::{ActorOptions, Priority};
/// use heph_rt::RuntimeRef;
/// # use heph_rt::{Runtime, ThreadLocal};
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
///     // Adding the server is the same as in the example above.
///     let new_actor = actor_fn(conn_actor);
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let server = Server::new(address, conn_supervisor, new_actor, ActorOptions::default())?;
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     let server_ref = runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///
///     // Because the server is just another actor we can send it messages. Here
///     // we'll send it a terminate message so it will gracefully shutdown.
///     server_ref.try_send(Terminate).unwrap();
///
///     Ok(())
/// }
///
/// // NOTE: `main`, `ServerSupervisor`, `conn_supervisor` and `conn_actor` are the same as
/// // in the previous example.
/// # /// Our supervisor for the server.
/// # #[derive(Copy, Clone, Debug)]
/// # struct ServerSupervisor;
/// #
/// # impl<NA> heph::Supervisor<NA> for ServerSupervisor
/// # where
/// #     NA: heph::NewActor<Argument = (), Error = io::Error>,
/// #     NA::Actor: heph::Actor<Error = ServerError<!>>,
/// # {
/// #     fn decide(&mut self, err: ServerError<!>) -> SupervisorStrategy<()> {
/// #         match err {
/// #             // When we hit an error accepting a connection we'll drop the old
/// #             // listener and create a new one.
/// #             ServerError::Accept(err) => {
/// #                 log::error!("error accepting new connection: {err}");
/// #                 SupervisorStrategy::Restart(())
/// #             }
/// #             // Async function never return an error creating a new actor.
/// #             ServerError::NewActor(err) => err,
/// #         }
/// #     }
/// #
/// #     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
/// #         log::error!("failed to restart listener, trying again: {err}");
/// #         SupervisorStrategy::Restart(())
/// #     }
/// #
/// #     fn second_restart_error(&mut self, err: io::Error) {
/// #         log::error!("failed to restart listener a second time, stopping it: {err}");
/// #     }
/// # }
/// #
/// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
/// #     log::error!("error handling connection: {err}");
/// #     SupervisorStrategy::Stop
/// # }
/// #
/// # async fn conn_actor(_: actor::Context<!, ThreadLocal>, conn: AsyncFd) -> io::Result<()> {
/// #     conn.send_all("Hello World").await?;
/// #     Ok(())
/// # }
/// ```
///
/// This example is similar to the first example, but runs the server actor as
/// thread-safe actor. *It's recommended to run the server as thread-local
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
/// use heph_rt::net::{Server, ServerError};
/// use heph_rt::spawn::options::{ActorOptions, Priority};
/// use heph_rt::{self as rt, Runtime, ThreadSafe};
///
/// fn main() -> Result<(), rt::Error> {
///     let mut runtime = Runtime::new()?;
///
///     // The address to listen on.
///     let address = "127.0.0.1:7890".parse().unwrap();
///     // Create our server. We'll use the default actor options.
///     let new_actor = actor_fn(conn_actor);
///     let server = Server::new(address, conn_supervisor, new_actor, ActorOptions::default())
///         .map_err(rt::Error::setup)?;
///
///     let options = ActorOptions::default().with_priority(Priority::LOW);
///     # let actor_ref =
///     runtime.try_spawn(ServerSupervisor, server, (), options)
///         .map_err(rt::Error::setup)?;
///     # actor_ref.try_send(Terminate).unwrap();
///
///     runtime.start()
/// }
///
/// // NOTE: `ServerSupervisor`, `conn_supervisor` and `conn_actor` are the same as
/// // in the previous example.
/// # /// Our supervisor for the server.
/// # #[derive(Copy, Clone, Debug)]
/// # struct ServerSupervisor;
/// #
/// # impl<NA> heph::Supervisor<NA> for ServerSupervisor
/// # where
/// #     NA: heph::NewActor<Argument = (), Error = io::Error>,
/// #     NA::Actor: heph::Actor<Error = ServerError<!>>,
/// # {
/// #     fn decide(&mut self, err: ServerError<!>) -> SupervisorStrategy<()> {
/// #         match err {
/// #             // When we hit an error accepting a connection we'll drop the old
/// #             // listener and create a new one.
/// #             ServerError::Accept(err) => {
/// #                 log::error!("error accepting new connection: {err}");
/// #                 SupervisorStrategy::Restart(())
/// #             }
/// #             // Async function never return an error creating a new actor.
/// #             ServerError::NewActor(err) => err,
/// #         }
/// #     }
/// #
/// #     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
/// #         log::error!("failed to restart listener, trying again: {err}");
/// #         SupervisorStrategy::Restart(())
/// #     }
/// #
/// #     fn second_restart_error(&mut self, err: io::Error) {
/// #         log::error!("failed to restart listener a second time, stopping it: {err}");
/// #     }
/// # }
/// #
/// #
/// # /// `conn_actor`'s supervisor.
/// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
/// #     log::error!("error handling connection: {err}");
/// #     SupervisorStrategy::Stop
/// # }
/// #
/// # /// The actor responsible for a single connection.
/// # async fn conn_actor(_: actor::Context<!, ThreadSafe>, conn: AsyncFd) -> io::Result<()> {
/// #     conn.send_all(a10::io::StaticBuf::from("Hello World")).await?;
/// #     Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Server<S, NA, A = SocketAddr> {
    /// Socket we bind in `Server::new` to `address`, to return an error quickly
    /// if we can't create the socket or bind to the address. Used in the first
    /// call to `NewActor::new`.
    socket: Arc<Mutex<Option<std::os::fd::OwnedFd>>>,
    /// Address of the `socket`.
    address: A,
    /// Supervisor for all actors created by `NewActor`.
    supervisor: S,
    /// NewActor used to create an actor for each connection.
    new_actor: NA,
    /// Options used to spawn the actor.
    options: ActorOptions,
}

impl<S, NA, A> Server<S, NA, A> {
    /// Create a new server.
    ///
    /// Arguments:
    ///  * `address`: the address to listen on.
    ///  * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    ///  * `new_actor`: the [`NewActor`] implementation to start each actor, and
    ///  * `options`: the actor options used to spawn the new actors.
    #[allow(clippy::cast_possible_truncation)] // For all the libc constants.
    pub fn new(
        address: A,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<Server<S, NA, A>>
    where
        S: Supervisor<NA> + Clone + 'static,
        NA: NewActor<Argument = AsyncFd> + Clone + 'static,
        NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
        A: SocketAddress + Copy + fmt::Display,
    {
        let fd = bind_listener(address)?;
        // Using a port of 0 (for IP addresses) means the OS selects one for us.
        // However, we still consistently want to use the same port instead of
        // binding to a number of random ports. So get the local address to use
        // as address.
        let address = local_address(fd.as_raw_fd())?;
        Ok(Server {
            socket: Arc::new(Mutex::new(Some(fd))),
            address,
            supervisor,
            new_actor,
            options,
        })
    }

    /// Change the supervisor for the the actor that handles the connections.
    #[rustfmt::skip]
    pub fn map_supervisor<F, S2>(self, map: F) -> Server<S2, NA, A>
    where
        F: FnOnce(S) -> S2,
        S2: Supervisor<NA> + Clone + 'static,
        NA: NewActor,
    {
        let Server { socket, address, supervisor, new_actor, options } = self;
        let supervisor = map(supervisor);
        Server { socket, address, supervisor, new_actor, options }
    }

    /// Change the actor that handles the incoming connections.
    #[rustfmt::skip]
    pub fn map_actor<F, NA2>(self, map: F) -> Server<S, NA2, A>
    where
        F: FnOnce(NA) -> NA2,
        NA2: NewActor<Argument = AsyncFd> + Clone + 'static,
        NA2::RuntimeAccess: Access + Spawn<S, NA2, NA2::RuntimeAccess>,
    {
        let Server { socket, address, supervisor, new_actor, options } = self;
        let new_actor = map(new_actor);
        Server { socket, address, supervisor, new_actor, options }
    }

    /// Change the actor options for the actor that handles incoming connections.
    #[rustfmt::skip]
    pub fn map_actor_options<F>(self, map: F) -> Self
    where
        F: FnOnce(ActorOptions) -> ActorOptions,
    {
        let Server { socket, address, supervisor, new_actor, options } = self;
        let options = map(options);
        Server { socket, address, supervisor, new_actor, options }
    }

    /// Returns the address the server is bound to.
    pub fn local_addr(&self) -> &A {
        &self.address
    }
}

/// Create a new listener bound to `address`, but **not** listening using
/// blocking I/O.
fn bind_listener<A: SocketAddress>(address: A) -> io::Result<std::os::fd::OwnedFd> {
    let domain = Domain::for_address(&address);
    // SAFETY: a10::net::Domain has the same layout as the underlying type,
    // which is libc::c_int.
    let raw_domain: libc::c_int = unsafe { std::mem::transmute_copy(&domain) };
    let fd = syscall!(socket(raw_domain, libc::SOCK_STREAM, 0))?;
    // SAFETY: just created the socket, so it's valid.
    let socket = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

    if matches!(domain, Domain::IPV4 | Domain::IPV6) {
        // Allow other worker threads and processes to reuse the address and
        // port we're binding to. This allows for restarting the process without
        // dropping clients.
        let value: libc::c_int = true.into();
        let ptr = ptr::from_ref(&value).cast();
        let len = size_of::<libc::c_int>() as u32;
        _ = syscall!(setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            ptr,
            len
        ))?;
        _ = syscall!(setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            ptr,
            len
        ))?;
    }

    // Bind the socket.
    let addr = address.into_storage();
    // SAFETY: to call, unsafe to implement.
    let (ptr, len) = unsafe { A::as_ptr(&addr) };
    let _ = syscall!(bind(fd, ptr.cast(), len))?;

    Ok(socket)
}

fn local_address<A: SocketAddress>(fd: RawFd) -> io::Result<A> {
    let mut addr = MaybeUninit::uninit();
    // SAFETY: to call, unsafe to implement.
    let (ptr, mut len) = unsafe { A::as_mut_ptr(&mut addr) };
    let _ = syscall!(getsockname(fd, ptr.cast(), &raw mut len,))?;
    // SAFETY: OS initialised the address for us, so we can safely call init.
    Ok(unsafe { A::init(addr, len) })
}

impl<S, NA, A> NewActor for Server<S, NA, A>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
    A: SocketAddress + Copy + fmt::Display,
{
    type Message = ServerMessage;
    type Argument = ();
    type Actor = impl Future<Output = Result<(), ServerError<NA::Error>>>;
    type Error = io::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        (): Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let fd = if let Some(fd) = self.socket.lock().unwrap().take() {
            fd // Reuse the already created socket if we can.
        } else {
            bind_listener(self.address)? // Or create a new socket.
        };
        let listener = AsyncFd::new(fd, ctx.runtime_ref().sq());
        Ok(server(
            ctx,
            listener,
            self.address,
            self.supervisor.clone(),
            self.new_actor.clone(),
            self.options.clone(),
        ))
    }
}

#[allow(clippy::cast_possible_truncation)] // For setting of IncomingCpu.
async fn server<S, NA, A>(
    mut ctx: actor::Context<ServerMessage, NA::RuntimeAccess>,
    listener: AsyncFd,
    local: A,
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> Result<(), ServerError<NA::Error>>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
    A: fmt::Display,
{
    #[cfg(any(target_os = "android", target_os = "linux"))]
    if let Some(cpu) = ctx.runtime_ref().cpu() {
        if let Err(err) = listener
            .set_socket_option::<option::IncomingCpu>(cpu as u32)
            .await
        {
            log::warn!("failed to set CPU affinity on server, continuing anyway: {err}");
        }
    }
    listener
        .listen(libc::SOMAXCONN.cast_unsigned())
        .await
        .map_err(ServerError::Accept)?;
    log::trace!(address:% = local; "Server listening");

    let mut accept = listener.multishot_accept();
    let mut receive = ctx.receive_next();
    loop {
        match either(next(&mut accept), &mut receive).await {
            Ok(Some(Ok(stream))) => {
                log::trace!("Server accepted connection");
                drop(receive); // Can't double borrow `ctx`.
                #[cfg(any(target_os = "android", target_os = "linux"))]
                if let Some(cpu) = ctx.runtime_ref().cpu() {
                    if let Err(err) = stream
                        .set_socket_option::<option::IncomingCpu>(cpu as u32)
                        .await
                    {
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
                log::debug!("no more connections to accept in server, stopping");
                return Ok(());
            }
            Err(Ok(_)) => {
                log::debug!("Server received shutdown message, stopping");
                return Ok(());
            }
            Err(Err(NoMessages)) => {
                log::debug!("All actor references to server dropped, stopping");
                return Ok(());
            }
        }
    }
}
