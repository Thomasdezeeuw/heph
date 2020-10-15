//! Relay message over the network.
//!
//! See [`RemoteRelay`].

// TODO:
// * Add a way to shut down the `remote_relay` actor cleanly.
// * Add RPC support.

use std::fmt;
use std::future::Future;
use std::io::{self, Cursor};
use std::net::SocketAddr;

use futures_util::future::{select, Either};
use log::{error, warn};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::net::udp::UdpSocket;
use crate::rt::{ActorOptions, Runtime, RuntimeAccess};
use crate::{actor, ActorRef, SupervisorStrategy};

/// The `RemoteRelay` is an actor that relays messages to actor(s) on remote
/// nodes. It allows actors to communicate using `ActorRef`s, transparently
/// sending the message of the network.
///
/// Only a single relay actor has be started per process, it can route messages
/// from multiple remote actors to one or more local actors. The
/// [`MessageRoute`] trait determines how the messages should be routed. The
/// simplest implementation of `MessageRoute` would be to send all messages to a
/// single actor (this is done by [`Relay`] type). However it can also be made
/// more complex, for example starting a new actor for each message or routing
/// different messages to different actors.
///
/// # Notes
///
/// The messages are send using UDP. This has two limitations:
///
/// * The size of a single message is limited to theoretical limit of roughly
///   65,000 bytes. However the larger the message, the larger the chance of
///   packet fragmenting (at low levels of the network stack), which in turns
///   increases the change of the message not being delivered. Its advisable to
///   keep message small (~500 bytes) for both increased delivery probability
///   and performance.
/// * Delivery of the message is not guaranteed, *but the same is true for any
///   actor reference*. To ensure delivery use [RPC] to acknowledge when a
///   message is successfully processed by the remote actor.
///
/// When sending large messages you might consider setting up a [`TcpStream`]
/// instead.
///
/// [RPC]: crate::actor_ref::rpc
/// [`TcpStream`]: crate::net::TcpStream
/*
///
/// # Examples
///
/// Simple example that relays messages from remote actors to a local actor.
///
/// ```
/// #![feature(never_type)]
///
/// use heph::{actor, rt, Runtime, ActorOptions};
/// use heph::net::relay::{RemoteRelay, Relay};
/// use heph::actor::context::ThreadSafe;
/// use heph::supervisor::NoSupervisor;
///
/// # fn main() -> Result<(), rt::Error<std::io::Error>> {
/// let local_address = "127.0.0.1:9001".parse().unwrap();
/// // Let's pretend this on a remote node.
/// let remote_address = "127.0.0.1:9002".parse().unwrap();
///
/// let mut runtime = Runtime::new().map_err(rt::Error::map_type)?;
///
/// // Spawn our local actor.
/// let local_actor = local_actor as fn(_) -> _;
/// let options = ActorOptions::default();
/// let local_actor_ref = runtime.spawn(NoSupervisor, local_actor, (), options);
///
/// // Create a router that relays all remote messages to our local actor.
/// let router = Relay::to(local_actor_ref);
///
/// // FIXME: remove.
/// fn is_send_sync<T: Send + Sync>() { }
/// is_send_sync::<Relay<String>>();
/// is_send_sync::<RemoteRelay<String>>();
/// is_send_sync::<heph::net::relay::routers::SendMessage<'_, String>>();
///
/// // Spawn our remote relay actor.
/// let options = ActorOptions::default();
/// let remote_relay = RemoteRelay::bind(&mut runtime, local_address, router, options)?;
///
/// /*
/// // Create a actor reference to a remote actor (also created via RemoteRelay).
/// let remote_actor_ref = remote_relay.create_ref(remote_address);
/// // Which can be used like any other `ActorRef`.
/// remote_actor_ref.send("Hello world".to_owned());
/// */
/// drop(remote_relay);
///
/// runtime.start().map_err(rt::Error::map_type)
/// # }
///
/// // Our local actor.
/// async fn local_actor(mut ctx: actor::Context<String, ThreadSafe>) -> Result<(), !> {
///     loop {
///         let msg = ctx.receive_next().await;
///         println!("received message: {}", msg);
/// #       assert_eq!(&*msg, "Hello world");
/// #       return Ok(());
///     }
/// }
/// ```
*/
#[derive(Debug, Clone)]
pub struct RemoteRelay<M> {
    actor_ref: ActorRef<(M, SocketAddr)>,
}

impl<M> RemoteRelay<M> {
    /// Start a new remote relay actor, binding to `local_address`.
    ///
    /// The remote relay actor will relay all messages it receives (see
    /// [`RemoteRelay::create_ref`]) to remote actors. It will also listen on
    /// `local_address` for incoming messages and route them use `router`.
    pub fn bind<R, In>(
        runtime: &mut Runtime,
        local_address: SocketAddr,
        router: R,
        options: ActorOptions,
    ) -> io::Result<RemoteRelay<M>>
    where
        M: Serialize + Send + Sync + 'static,
        R: MessageRoute<In> + Send + Sync + 'static,
        R::Route: Send + Sync,
        In: DeserializeOwned + Send + Sync + 'static,
    {
        let mut socket = UdpSocket::new(local_address)?;

        // TODO: support restarting. Requires a way to get the Router back from
        // the actor.
        let supervisor = |err| {
            error!("`RemoteRelay` failed, stopping it: {}", err);
            SupervisorStrategy::Stop
        };
        let remote_relay = remote_relay as fn(_, _, _) -> _;
        let res = actor::private::Spawn::try_spawn_setup(
            runtime,
            supervisor,
            remote_relay,
            // Register the socket, ensuring it runs when the socket is ready.
            |ctx| socket.register(ctx).map(|()| (socket, router)),
            options,
        );

        use actor::private::AddActorError;
        match res {
            Ok(actor_ref) => Ok(RemoteRelay { actor_ref }),
            // async functions never return an error in creating the actor.
            Err(AddActorError::<!, _>::NewActor(_)) => unreachable!(),
            Err(AddActorError::ArgFn(err)) => Err(err),
        }
    }

    /// Create a reference to a `RemoteRelay` listening on `remote_address`.
    ///
    /// Any messages send to the returned actor reference will be send to an
    /// actor listening on `remote_actor`.
    pub fn create_ref(&mut self, remote_address: SocketAddr) -> ActorRef<M> {
        self.actor_ref.relay_to(remote_address).unwrap()
    }
}

/// Actor that handles remote messages
///
/// `In`coming and `Out`going remote messages.
///
/// It receives `Out`going messages from it's inbox and sends them to a remote
/// actor using the [`UdpSocket`]. Any `In`coming message on the same socket
/// will be routed using the `R`outer.
async fn remote_relay<Out, K, R, In>(
    mut ctx: actor::Context<(Out, SocketAddr), K>,
    mut socket: UdpSocket,
    mut router: R,
) -> io::Result<()>
where
    actor::Context<(Out, SocketAddr), K>: RuntimeAccess,
    Out: Serialize,
    R: MessageRoute<In>,
    In: DeserializeOwned,
{
    // Buffer with the maximum size of a UDP packet, ~65kB.
    let mut buf = [0u8; (1 << 16) - (1 << 9)];

    loop {
        match select(ctx.receive_next(), socket.recv_from(&mut buf)).await {
            // Received an outgoing message we want to relay to a remote actor.
            Either::Left(((msg, target), _)) => {
                send_message(&mut socket, &mut buf, &msg, target).await?
            }
            // Received an incoming packet.
            Either::Right((Ok((n, source)), _)) => {
                route_message(&mut router, &mut buf[..n], source).await?
            }
            // Error receiving a packet.
            Either::Right((Err(err), _)) => return Err(err),
        }
    }
}

/// Send a `msg` to a remote actor at `target` address, using `socket`.
async fn send_message<M>(
    socket: &mut UdpSocket,
    buf: &mut [u8],
    msg: &M,
    target: SocketAddr,
) -> io::Result<()>
where
    M: Serialize,
{
    // Serialise the message to our buffer first.
    let mut wbuf = Cursor::new(&mut *buf);
    let n = match serde_cbor::to_writer(&mut wbuf, msg) {
        Ok(()) => wbuf.position() as usize,
        Err(err) => {
            warn!("error serialising message in `RemoteRelay`: {}", err);
            // Don't want to stop the actor for this.
            return Ok(());
        }
    };

    socket
        .send_to(&buf[..n], target)
        .await
        .and_then(|bytes_send| {
            if bytes_send == n {
                Ok(())
            } else {
                // Should never happen, but just in case.
                Err(io::ErrorKind::WriteZero.into())
            }
        })
}

/// Routes a message in `buf` using `router`.
///
/// Returns an error if the message can't be routed. Error deserialising the
/// message in `buf` are only logged using `warn!`.
async fn route_message<R, M>(router: &mut R, buf: &mut [u8], source: SocketAddr) -> io::Result<()>
where
    R: MessageRoute<M>,
    M: DeserializeOwned,
{
    let mut de = serde_cbor::Deserializer::from_mut_slice(buf);
    match Deserialize::deserialize(&mut de) {
        Ok(msg) => {
            if let Err(err) = router.route(msg, source).await {
                let msg = format!("failed to route message: {}", err);
                return Err(io::Error::new(io::ErrorKind::Other, msg));
            }

            if let Err(err) = de.end() {
                warn!(
                    "trailing data in message in `RemoteRelay`: {}: remote_address=\"{}\"",
                    err, source
                );
            }
        }
        Err(err) => {
            warn!(
                "error deserialising message in `RemoteRelay`: {}: remote_address=\"{}\"",
                err, source
            );
        }
    }
    Ok(())
}

/// Trait that determines how to route a message.
///
/// See [`RemoteRelay`].
pub trait MessageRoute<M> {
    /// Possible error returned by [routing].
    ///
    /// If no error is possible the never type (`!`) can be used.
    ///
    /// [routing]: MessageRoute::route
    type Error: fmt::Display;

    /// [`Future`] that determines how to route a message, see [`route`].
    ///
    /// [`route`]: MessageRoute::route
    // FIXME: add lifetime to `Route` to allow it to borrow from `self`, but
    // that requires GAT (generic_associated_types,
    // https://github.com/rust-lang/rust/issues/44265).
    type Route: Future<Output = Result<(), Self::Error>> + Unpin;

    /// Route a `msg` from `source` address to the correct actor.
    ///
    /// This method must return a [`Future`], but not all routing requires the
    /// use of a `Future`, in that case [`ready`] can be used.
    ///
    /// [`ready`]: std::future::ready
    fn route(&mut self, msg: M, source: SocketAddr) -> Self::Route;
}

pub mod routers {
    //! Various [`MessageRoute`]rs.

    use std::future::Future;
    use std::future::{ready, Ready};
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use crate::actor_ref::{ActorRef, SendError};
    use crate::net::relay::MessageRoute;

    /// [`MessageRoute`] implementation that routes all messages to a single actor.
    #[derive(Debug)]
    pub struct Relay<M> {
        actor_ref: ActorRef<M>,
    }

    impl<M> Relay<M> {
        /// Relay all remote messages to the `actor_ref`.
        pub const fn to(actor_ref: ActorRef<M>) -> Relay<M> {
            Relay { actor_ref }
        }
    }

    impl<Msg, M> MessageRoute<Msg> for Relay<M>
    where
        Msg: Into<M>,
        M: 'static + Unpin,
    {
        type Error = SendError;
        type Route = SendMessage<M>;

        fn route(&mut self, msg: Msg, _: SocketAddr) -> Self::Route {
            SendMessage {
                // FIXME: add a lifetime to `MessageRoute::Route` and borrow the
                // `ActorRef` here.
                actor_ref: self.actor_ref.clone(),
                msg: Some(msg.into()),
            }
        }
    }

    /// Future behind [`Relay::route`].
    #[derive(Debug)]
    pub struct SendMessage<M> {
        actor_ref: ActorRef<M>,
        msg: Option<M>,
    }

    impl<M> Future for SendMessage<M>
    where
        M: Unpin,
    {
        type Output = Result<(), SendError>;

        fn poll(self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
            let this = Pin::into_inner(self);
            let msg = this
                .msg
                .take()
                .expect("polled `SendMessage` after completion.");
            Poll::Ready(this.actor_ref.send(msg))
        }
    }

    /// Router that drops all incoming messages.
    #[derive(Debug)]
    pub struct Drop;

    impl<M> MessageRoute<M> for Drop
    where
        M: 'static + Unpin,
    {
        type Error = !;
        type Route = Ready<Result<(), Self::Error>>;

        fn route(&mut self, _: M, _: SocketAddr) -> Self::Route {
            ready(Ok(()))
        }
    }
}

#[doc(no_inline)]
pub use routers::{Drop, Relay};
