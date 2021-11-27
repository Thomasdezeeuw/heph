//! Relay messages over the network.
//!
//! The remote relay is an actor that relays messages to actor(s) on remote
//! nodes. It allows actors to communicate using `ActorRef`s, transparently
//! sending the message of the network.
//!
//! Only a single relay actor has be started per process, it can route messages
//! from multiple remote actors to one or more local actors. The [`Route`] trait
//! determines how the messages should be routed. The simplest implementation of
//! `Route` would be to send all messages to a single actor (this is done by
//! [`Relay`] type). However it can also be made more complex, for example
//! starting a new actor for each message or routing different messages to
//! different actors.
//!
//! When sending large messages or streaming large amounts of data you should
//! consider setting up a [`TcpStream`] instead.
//!
//! [`TcpStream`]: heph::net::TcpStream
//!
//! # Examples
//!
//! Simple example that relays messages from remote actors to a local actor.
//!
//! ```
//! #![feature(never_type)]
//!
//! use std::net::SocketAddr;
//!
//! use heph::rt::{self, Runtime};
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, restart_supervisor, ActorOptions, ActorRef};
//! use heph_remote::net_relay::{self, Relay, UdpRelayMessage};
//!
//! # fn main() -> Result<(), rt::Error> {
//! # return Ok(()); // Don't want to send any packets.
//! let local_address = "127.0.0.1:9001".parse().unwrap();
//! // Let's pretend this on a remote node.
//! let remote_address = "127.0.0.1:9002".parse().unwrap();
//!
//! let mut runtime = Runtime::new()?;
//!
//! // Spawn our local actor.
//! let local_actor = local_actor as fn(_) -> _;
//! let local_actor_ref = runtime.spawn(NoSupervisor, local_actor, (), ActorOptions::default());
//!
//! let supervisor = RelaySupervisor::new(local_address);
//! // Create a router that relays all incoming messages to our local actor.
//! let router: Relay<String> = Relay::to(local_actor_ref);
//! let relay = net_relay::Config::default().udp().json().route(router);
//! // Spawn our remote relay actor.
//! let remote_ref: ActorRef<UdpRelayMessage<String>> =
//!     runtime.spawn(supervisor, relay, local_address, ActorOptions::default());
//!
//! // For convenience we can map the actor ref to an easier to use type.
//! let remote_ref: ActorRef<Outgoing> =
//!     remote_ref.map_fn(move |msg: Outgoing| UdpRelayMessage::Relay {
//!         message: msg.0,
//!         target: remote_address,
//!     });
//!
//! // Now the actor reference can be used like any other and it will deliver
//! // the message to the actor across the network (assuming someone is
//! // listening of course).
//! remote_ref
//!     .try_send(Outgoing("Hello world!".to_owned()))
//!     .unwrap();
//!
//! // Dropping all reference to the relay actor will stop it.
//! drop(remote_ref);
//! // If you want keep listening for remote messages, even though you're not
//! // sending any of your own, you'll need to keep `remote_ref` alive at least
//! // until `runtime.start()` below returns.
//!
//! runtime.start()
//! # }
//!
//! restart_supervisor!(RelaySupervisor, "relay actor", SocketAddr);
//!
//! struct Outgoing(String);
//!
//! // Our local actor.
//! async fn local_actor<RT>(mut ctx: actor::Context<String, RT>)
//! where
//!     RT: rt::Access,
//! {
//!     while let Ok(msg) = ctx.receive_next().await {
//!         println!("received message: {}", msg);
//!     }
//! }
//! ```

use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::{fmt, io};

use heph::actor::{self, Actor, NewActor};
use heph::rt;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod routers;
mod udp;
mod uuid;

#[doc(no_inline)]
pub use routers::{Relay, RelayGroup};
#[doc(inline)]
pub use udp::UdpRelayMessage;

/// Use a [TCP] connection.
///
/// This is mainly indented for connections between datacentres, for
/// intra-datacentre connection [`Udp`] might be faster, without extra loss of
/// messages.
///
/// Note that "reliable" here refers to the connection type, no guarantees are
/// provided about message delivery.
///
/// [TCP]: heph::net::tcp
pub enum Tcp {}

/// Use a [UDP] connection.
///
/// This is a faster alternative to [`Tcp`] for intra-datacentre communication.
/// For connections outside datacentre or local networks [`Tcp`] might be more,
/// well, reliable.
///
/// [UDP]: heph::net::udp
///
/// # Notes
///
/// As the messages are send using UDP it has two limitations:
///
/// * The size of a single message is limited to theoretical limit of roughly
///   65,000 bytes. However the larger the message, the larger the chance of
///   packet fragmenting (at low levels of the network stack), which in turns
///   increases the change of the message not being delivered. Its advisable to
///   keep message small (< 2kb) for both increased delivery probability and
///   performance.
/// * Delivery of the message is not guaranteed, *but the same is true for any
///   actor reference*. To ensure delivery use [RPC] to acknowledge when a
///   message is successfully processed by the remote actor.
///
/// [RPC]: heph::actor_ref::rpc
pub enum Udp {}

/// Use JSON serialisation.
#[cfg(any(feature = "json"))]
pub enum Json {}

/// Configuration for the net relay.
///
/// The following configuration opotions are available:
///  * `R`: [`Route`]r to route incoming message.
///  * `CT`: contection to use, either [`Udp`] or [`Tcp`].
///  * `S`: serialisation format, currently only [`Json`] is supported.
///  * `Out`: outgoing message type.
///  * `In`: incoming message type (those that are routed by `R`).
///  * `RT`: [`rt::Access`] type used by the spawned actor.
pub struct Config<R, CT, S, Out, In, RT> {
    /// How to route incoming messages.
    router: R,
    /// Type of connection to use.
    conection_type: PhantomData<CT>,
    /// Type of serialisation to use.
    serialisation: PhantomData<S>,
    /// Types needed in the `NewActor` implementation.
    _types: PhantomData<(Out, In, RT)>,
}

impl<Out, In, RT> Config<(), (), (), Out, In, RT> {
    /// Create a default configuration.
    ///
    /// This still needs the following configuration options to be set (all set
    /// to `()`):
    ///  * `R`: [`Route`]r,
    ///  * `CT`: Connection type,
    ///  * `S`: serialisation format.
    pub const fn default() -> Config<(), (), (), Out, In, RT> {
        Config {
            router: (),
            conection_type: PhantomData,
            serialisation: PhantomData,
            _types: PhantomData,
        }
    }
}

impl<CT, S, Out, In, RT> Config<(), CT, S, Out, In, RT> {
    /// Use the `router` to route incoming messages.
    pub fn route<R>(self, router: R) -> Config<R, CT, S, Out, In, RT>
    where
        R: Route<In> + Clone,
    {
        Config {
            router,
            conection_type: self.conection_type,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }
}

impl<R, S, Out, In, RT> Config<R, (), S, Out, In, RT> {
    /// Use a [`Tcp`] connection.
    pub fn tcp(self) -> Config<R, Tcp, S, Out, In, RT> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }

    /// Use a [`Udp`] connection.
    pub fn udp(self) -> Config<R, Udp, S, Out, In, RT> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }
}

impl<R, CT, Out, In, RT> Config<R, CT, (), Out, In, RT> {
    /// Use [`Json`] serialisation.
    #[cfg(any(feature = "json"))]
    pub fn json(self) -> Config<R, CT, Json, Out, In, RT> {
        Config {
            router: self.router,
            conection_type: self.conection_type,
            serialisation: PhantomData,
            _types: PhantomData,
        }
    }
}

impl<R, S, Out, In, RT> NewActor for Config<R, Udp, S, Out, In, RT>
where
    R: Route<In> + Clone,
    In: DeserializeOwned,
    S: Serde,
    RT: rt::Access,
    Out: Serialize,
{
    type Message = UdpRelayMessage<Out>;
    type Argument = SocketAddr;
    type Actor = impl Actor<Error = io::Error>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        local_address: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(udp::remote_relay::<S, Out, In, R, RT>(
            ctx,
            local_address,
            self.router.clone(),
        ))
    }
}

mod private {
    //! Private in public hack.

    use std::fmt;

    use serde::de::DeserializeOwned;
    use serde::Serialize;

    use super::Json;

    pub trait Serde {
        type Error: fmt::Display;

        fn from_slice<'a, T>(buf: &'a [u8]) -> Result<T, Self::Error>
        where
            T: DeserializeOwned;

        fn to_buf<'a, T>(buf: &mut Vec<u8>, msg: &'a T) -> Result<(), Self::Error>
        where
            T: ?Sized + Serialize;
    }

    #[cfg(any(feature = "json"))]
    impl Serde for Json {
        type Error = serde_json::Error;

        fn from_slice<'a, T>(buf: &'a [u8]) -> Result<T, Self::Error>
        where
            T: DeserializeOwned,
        {
            serde_json::from_slice(buf)
        }

        fn to_buf<'a, T>(buf: &mut Vec<u8>, msg: &'a T) -> Result<(), Self::Error>
        where
            T: ?Sized + Serialize,
        {
            serde_json::to_writer(buf, msg)
        }
    }
}

use private::Serde;

/// Trait that determines how to route a message.
pub trait Route<M> {
    /// [`Future`] that determines how to route a message, see [`route`].
    ///
    /// [`route`]: Route::route
    type Route<'a>: Future<Output = Result<(), Self::Error>>;

    /// Error returned by [routing]. This error is considered fatal for the
    /// relay actor, meaning it will be stopped.
    ///
    /// If no error is possible the never type (`!`) can be used.
    ///
    /// [routing]: Route::route
    type Error: fmt::Display;

    /// Route a `msg` from `source` address to the correct destination.
    ///
    /// This method must return a [`Future`], but not all routing requires the
    /// use of a `Future`, in that case [`ready`] can be used.
    ///
    /// [`ready`]: std::future::ready
    fn route<'a>(&'a mut self, msg: M, source: SocketAddr) -> Self::Route<'a>;
}
