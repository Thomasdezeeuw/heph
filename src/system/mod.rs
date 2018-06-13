//! TODO: docs

use actor::Actor;
use initiator::Initiator;

mod actor_process;
mod actor_ref;
mod builder;
mod process;
mod scheduler;

pub mod error;
pub mod options;

pub use self::actor_ref::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::ActorOptions;

use self::error::RuntimeError;
use self::process::ProcessId;

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

    /// Queue an process to run.
    fn queue_process(&mut self, _id: ProcessId) {
        unimplemented!("ActorSystemRef.queue_process");
    }
}
