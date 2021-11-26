//! Relay messages over the network.

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;

use heph::actor::{self, Actor, NewActor};
use heph::rt;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod routers;
mod udp;

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
/// Note however that UDP connections can only send message up to ~65kb in
/// length (2^16 - packet and protocol) overhead.
///
/// [UDP]: heph::net::udp
pub enum Udp {}

/// Use JSON serialisation.
#[cfg(any(feature = "json"))]
pub enum Json {}

/// Configuration for the net relay.
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

impl<R, CT, S, Out, In, RT> Config<R, CT, S, Out, In, RT> {
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

    /// Use the `router` to route incoming messages.
    pub fn route(self, router: R) -> Config<R, CT, S, Out, In, RT> {
        Config {
            router,
            conection_type: self.conection_type,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }

    /// Use a [`Tcp`] connection.
    pub fn tcp(self) -> Config<R, Tcp, S, Out, In, RT> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }

    /// Use an [`Udp`] connection.
    pub fn udp(self) -> Config<R, Udp, S, Out, In, RT> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
            _types: PhantomData,
        }
    }

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
    type Actor = impl Actor;
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
