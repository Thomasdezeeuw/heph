//! Relay messages over the network.
//!
//! The remote relay is an actor that relays messages to actor(s) on remote
//! nodes. The main purpose it to abstract away the network from the
//! communication between actors. It allows actors to communicate using
//! `ActorRef`s, transparently sending the message of the network.
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
//! [`TcpStream`]: heph_rt::net::TcpStream
//!
//! # Examples
//!
//! Simple example that relays messages from remote actors to a local actor.
//!
#![cfg_attr(feature = "json", doc = "```")]
#![cfg_attr(not(feature = "json"), doc = "```rust,ignore")]
//! #![feature(never_type)]
//!
//! use std::net::SocketAddr;
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::actor::{self, actor_fn};
//! use heph::{restart_supervisor, ActorRef};
//! use heph_remote::net_relay::{self, Relay, UdpRelayMessage};
//! use heph_rt::spawn::ActorOptions;
//! use heph_rt::{self as rt, Runtime};
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
//! async fn local_actor<RT>(mut ctx: actor::Context<String, RT>)
//! where
//!     RT: rt::Access,
//! {
//!     while let Ok(msg) = ctx.receive_next().await {
//!         println!("received message: {msg}");
//!     }
//! }
//! let local_actor = actor_fn(local_actor);
//! let local_actor_ref = runtime.spawn(NoSupervisor, local_actor, (), ActorOptions::default());
//!
//! // Next we're going to spawn our net relay actor.
//! // First it needs a supervisor.
//! restart_supervisor!(RelaySupervisor, "relay actor", SocketAddr);
//! let supervisor = RelaySupervisor::new(local_address);
//! // It needs a way to route all incoming messages, here we're direct them to
//! // our local actor using the `local_actor_ref`.
//! let router: Relay<String> = Relay::to(local_actor_ref);
//! // Configure the net relay.
//! let relay = net_relay::Config::new().udp().json().route(router);
//! // Finally spawn it like a normal actor.
//! let remote_ref: ActorRef<UdpRelayMessage<String>> =
//!     runtime.spawn(supervisor, relay, local_address, ActorOptions::default());
//!
//! // For convenience we can map the actor ref to an easier to use type.
//! let remote_ref: ActorRef<String> = remote_ref.map_fn(move |msg| UdpRelayMessage::Relay {
//!     message: msg,
//!     target: remote_address,
//! });
//!
//! // Now the actor reference can be used like any other and it will deliver
//! // the message to the actor across the network (assuming someone is
//! // listening of course).
//! remote_ref.try_send("Hello world!").unwrap();
//!
//! // Dropping all reference to the relay actor will stop it.
//! drop(remote_ref);
//! // If you want keep listening for remote messages, even though you're not
//! // sending any of your own, you'll need to keep `remote_ref` alive at least
//! // until `runtime.start()` returns below.
//!
//! runtime.start()
//! # }
//! ```

use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::{fmt, io};

use heph::actor::{self, Actor, NewActor};
use heph_rt as rt;
use serde::de::{self, Deserialize, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeStruct, Serializer};

pub mod routers;
mod tcp;
mod udp;
mod uuid;

use uuid::Uuid;

#[doc(no_inline)]
pub use routers::{Relay, RelayGroup};
#[doc(inline)]
pub use tcp::RelayMessage;
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
/// [TCP]: heph_rt::net::tcp
#[allow(missing_debug_implementations)]
#[allow(clippy::empty_enum)]
pub enum Tcp {}

/// Use a [UDP] connection.
///
/// This is a faster alternative to [`Tcp`] for intra-datacentre communication.
/// For connections outside datacentre or local networks [`Tcp`] might be more,
/// well, reliable.
///
/// [UDP]: heph_rt::net::udp
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
#[allow(missing_debug_implementations)]
#[allow(clippy::empty_enum)]
pub enum Udp {}

/// Use JSON serialisation.
#[cfg(feature = "json")]
#[allow(missing_debug_implementations)]
#[allow(clippy::empty_enum)]
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
///
/// See the [module documentation] for an example.
///
/// [module documentation]: crate::net_relay#examples
#[derive(Debug)]
pub struct Config<R, CT, S, Out, In, RT> {
    /// How to route incoming messages.
    router: R,
    /// Type of connection to use.
    connection_type: PhantomData<CT>,
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
    ///  * `CT`: connection type,
    ///  * `S`: serialisation format.
    pub const fn new() -> Config<(), (), (), Out, In, RT> {
        Config {
            router: (),
            connection_type: PhantomData,
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
            connection_type: self.connection_type,
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
            connection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }

    /// Use a [`Udp`] connection.
    pub fn udp(self) -> Config<R, Udp, S, Out, In, RT> {
        Config {
            router: self.router,
            connection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }
}

impl<R, CT, Out, In, RT> Config<R, CT, (), Out, In, RT> {
    /// Use [`Json`] serialisation.
    #[cfg(feature = "json")]
    pub fn json(self) -> Config<R, CT, Json, Out, In, RT> {
        Config {
            router: self.router,
            connection_type: self.connection_type,
            serialisation: PhantomData,
            _types: PhantomData,
        }
    }
}

impl<R, S, Out, In, RT> NewActor for Config<R, Tcp, S, Out, In, RT>
where
    R: Route<In> + Clone,
    In: DeserializeOwned,
    S: Serde,
    RT: rt::Access,
    Out: Serialize,
{
    type Message = RelayMessage<Out>;
    type Argument = SocketAddr;
    type Actor = impl Actor<Error = io::Error>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        remote_address: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(tcp::remote_relay::<S, Out, In, R, RT>(
            ctx,
            remote_address,
            self.router.clone(),
        ))
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

impl<R, CT, S, Out, In, RT> Clone for Config<R, CT, S, Out, In, RT>
where
    R: Clone,
{
    #[allow(clippy::used_underscore_binding)]
    fn clone(&self) -> Config<R, CT, S, Out, In, RT> {
        Config {
            router: self.router.clone(),
            connection_type: self.connection_type,
            serialisation: self.serialisation,
            _types: self._types,
        }
    }

    #[allow(clippy::used_underscore_binding)]
    fn clone_from(&mut self, source: &Self) {
        self.router.clone_from(&source.router);
        self.connection_type.clone_from(&source.connection_type);
        self.serialisation.clone_from(&source.serialisation);
        self._types.clone_from(&source._types);
    }
}

mod private {
    //! Private in public hack.

    use std::fmt;

    use serde::de::DeserializeOwned;
    use serde::Serialize;

    #[cfg(feature = "json")]
    use super::Json;

    /// Trait that defined (de)serialisation.
    pub trait Serde {
        type Iter<'a, T>: DeIter<T>
        where
            T: DeserializeOwned;

        /// Error type wrapped into an `io::Error`.
        type Error: fmt::Display;

        /// Deserialise from the `buf`fer.
        fn from_slice<'a, T>(buf: &'a [u8]) -> Result<T, Self::Error>
        where
            T: DeserializeOwned;

        /// Serialise `msg` into the `buf`fer.
        fn to_buf<'a, T>(buf: &mut Vec<u8>, msg: &'a T) -> Result<(), Self::Error>
        where
            T: ?Sized + Serialize;

        /// Returns an iterator that deserialises messages from the `buf`fer.
        fn iter<'a, T>(buf: &'a [u8]) -> Self::Iter<'a, T>
        where
            T: DeserializeOwned;
    }

    /// Trait that defined an iterator for deserialised messages.
    pub trait DeIter<T>: Iterator<Item = Result<T, Self::Error>>
    where
        T: DeserializeOwned,
    {
        type Error: fmt::Display;

        /// Returns the number of bytes used.
        fn byte_offset(&self) -> usize;
    }

    #[cfg(feature = "json")]
    impl Serde for Json {
        type Iter<'a, T> = serde_json::StreamDeserializer<'a, serde_json::de::SliceRead<'a>, T>
            where T: DeserializeOwned;
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

        fn iter<'a, T>(buf: &'a [u8]) -> Self::Iter<'a, T>
        where
            T: DeserializeOwned,
        {
            serde_json::StreamDeserializer::new(serde_json::de::SliceRead::new(buf))
        }
    }

    #[cfg(feature = "json")]
    impl<'de, R, T> DeIter<T> for serde_json::StreamDeserializer<'de, R, T>
    where
        T: DeserializeOwned,
        R: serde_json::de::Read<'de>,
    {
        type Error = serde_json::Error;

        fn byte_offset(&self) -> usize {
            serde_json::StreamDeserializer::byte_offset(self)
        }
    }
}

use private::{DeIter, Serde};

/// Trait that determines how to route a message.
pub trait Route<M> {
    /// [`Future`] that determines how to route a message, see [`route`].
    ///
    /// [`route`]: Route::route
    type Route<'a>: Future<Output = Result<(), Self::Error>>
    where
        Self: 'a;

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

/// Message type used in communicating.
struct Message<M> {
    uuid: Uuid,
    msg: M,
}

// NOTE: manually implementing this instead of deriving to not pull in a bunch
// of dependencies.
impl<'de, M> Deserialize<'de> for Message<M>
where
    M: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Uuid,
            Msg,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("`uuid` or `message`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "uuid" => Ok(Field::Uuid),
                            "message" => Ok(Field::Msg),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct MessageVisitor<M>(PhantomData<M>);

        impl<'de, M> Visitor<'de> for MessageVisitor<M>
        where
            M: Deserialize<'de>,
        {
            type Value = Message<M>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("struct Message")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Message<M>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut uuid = None;
                let mut msg = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Uuid => {
                            if uuid.is_some() {
                                return Err(de::Error::duplicate_field("uuid"));
                            }
                            uuid = Some(map.next_value()?);
                        }
                        Field::Msg => {
                            if msg.is_some() {
                                return Err(de::Error::duplicate_field("message"));
                            }
                            msg = Some(map.next_value()?);
                        }
                    }
                }
                let uuid = uuid.ok_or_else(|| de::Error::missing_field("uuid"))?;
                let msg = msg.ok_or_else(|| de::Error::missing_field("message"))?;
                Ok(Message { uuid, msg })
            }
        }

        const FIELDS: &[&str] = &["uuid", "message"];
        deserializer.deserialize_struct("Message", FIELDS, MessageVisitor(PhantomData))
    }
}

impl<M> Serialize for Message<M>
where
    M: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Message", 2)?;
        state.serialize_field("uuid", &self.uuid)?;
        state.serialize_field("message", &self.msg)?;
        state.end()
    }
}
