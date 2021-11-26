//! Relay messages over the network.

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;

pub mod routers;

#[doc(no_inline)]
pub use routers::{Relay, RelayGroup};

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
pub enum Json {}

/// Configuration for the net relay.
pub struct Config<R, CT, S> {
    /// How to route incoming messages.
    router: R,
    /// Type of connection to use.
    conection_type: PhantomData<CT>,
    /// Type of serialisation to use.
    serialisation: PhantomData<S>,
}

impl<R, CT, S> Config<R, CT, S> {
    /// Create a default configuration.
    ///
    /// This still needs the connection type and serialisation format to be set
    /// before it can be used.
    pub const fn default() -> Config<(), (), ()> {
        Config {
            router: (),
            conection_type: PhantomData,
            serialisation: PhantomData,
        }
    }

    /// Use the `router` to route incoming messages.
    pub fn route(self, router: R) -> Config<R, CT, S> {
        Config {
            router,
            conection_type: self.conection_type,
            serialisation: self.serialisation,
        }
    }

    /// Use a [`Tcp`] connection.
    pub fn tcp(self) -> Config<R, Tcp, S> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
        }
    }

    /// Use a [`Udp`] connection.
    pub fn udp(self) -> Config<R, Udp, S> {
        Config {
            router: self.router,
            conection_type: PhantomData,
            serialisation: self.serialisation,
        }
    }

    /// Use [`Json`] serialisation.
    pub fn json(self) -> Config<R, CT, Json> {
        Config {
            router: self.router,
            conection_type: self.conection_type,
            serialisation: PhantomData,
        }
    }
}

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
