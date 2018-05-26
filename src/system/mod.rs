//! TODO: docs

use actor::Actor;

mod actor;

pub use self::actor::{ActorRef, ActorOptions, Priority, SendError, SendErrorReason, PRIORITY_MIN};

use self::actor::{ActorId};

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
}

impl ActorSystem {
    /// Add a new actor to the system.
    pub fn add_actor<'a, A>(&mut self, actor: A, options: ActorOptions) -> ActorRef<A>
        where A: Actor<'a>,
    {
        unimplemented!("ActorSystem.add_actor");
    }
}

// TODO: impl Default for `ActorSystem`.

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
    pub fn add_actor<'a, A>(&mut self, actor: A) -> ActorRef<A>
        where A: Actor<'a>,
    {
        unimplemented!("ActorSystemRef.add_actor");
    }

    /// Queue an actor to run.
    pub(crate) fn queue_actor(&mut self, id: ActorId) {
        unimplemented!("ActorSystemRef.queue_actor");
    }
}
