//! TODO: docs

use std::io;

use mio_st::poll::Poll;

use actor::Actor;
use initiator::Initiator;

mod actor_process;
mod builder;
mod process;
mod scheduler;

pub mod error;
pub mod options;

pub use self::actor_process::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::ActorOptions;

use self::actor_process::ActorProcess;
use self::error::RuntimeError;
use self::scheduler::Scheduler;
use self::process::{ProcessId, ProcessIdGenerator, ProcessPtr};

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
    scheduler: Scheduler,
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
    poll: Poll,
}

impl ActorSystem {
    /// Add a new actor to the system.
    // TODO: keep this in sync with `ActorSystemRef.add_actor`.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> io::Result<ActorRef<A::Message>>
        where A: Actor<'static> + 'static,
    {
        // TODO: return a different error then an `io::Error`.
        let pid = self.pid_gen.next();
        let process = ActorProcess::new(pid, actor, options, &mut self.poll)?;
        let actor_ref = process.create_ref();
        let process: ProcessPtr = Box::new(process);
        self.scheduler.add_process(process);
        Ok(actor_ref)
    }

    /// Run the system with the provided `initiators`.
    pub fn run<I>(self, initiators: &mut [I]) -> Result<(), RuntimeError>
        where I: Initiator,
    {
        unimplemented!("ActorSystem.run");
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
