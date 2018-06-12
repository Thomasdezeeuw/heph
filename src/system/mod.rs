//! TODO: docs

use actor::Actor;
use initiator::Initiator;

mod actor;
mod builder;
mod scheduler;
pub mod options;

pub use self::options::ActorOptions;

use self::actor::{ActorRef, SendError, SendErrorReason};
pub use self::builder::ActorSystemBuilder;

/// Unique id for each actor.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct ActorId(u64);

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
}

impl ActorSystem {
    /// Add a new actor to the system.
    // TODO: keep this in sync with `ActorSystemRef.add_actor`.
    pub fn add_actor<'a, A>(&mut self, actor: A, options: ActorOptions) -> ActorRef<A>
        where A: Actor<'a>,
    {
        unimplemented!("ActorSystem.add_actor");
    }

    /// Run the system with the provided `initiators`.
    pub fn run<I>(self, initiators: &mut [I]) -> Result<(), RuntimeError>
        where I: Initiator,
    {
        unimplemented!("ActorSystem.run");
    }
}

impl Default for ActorSystem {
    fn default() -> ActorSystem {
        ActorSystemBuilder::default().build()
    }
}

/// Error returned by running an `ActorSystem`.
#[derive(Debug)]
pub struct RuntimeError {
}

/// A reference to an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemRef {
}

impl ActorSystemRef {
    /// Add a new actor to the system.
    ///
    /// See [`ActorSystem.add_actor`].
    ///
    /// [`ActorSystem.add_actor`]: struct.ActorSystem.html#method.add_actor
    // TODO: keep this in sync with `ActorSystem.add_actor`.
    pub fn add_actor<'a, A>(&mut self, actor: A, options: ActorOptions) -> ActorRef<A>
        where A: Actor<'a>,
    {
        unimplemented!("ActorSystemRef.add_actor");
    }

    /// Queue an actor to run.
    pub(crate) fn queue_actor(&mut self, id: ActorId) {
        unimplemented!("ActorSystemRef.queue_actor");
    }
}
