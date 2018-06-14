//! TODO: docs

use std::io;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use mio_st::poll::Poll;
use mio_st::event::Events;

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
use self::scheduler::Scheduler;
use self::process::{ProcessId, ProcessIdGenerator, ProcessPtr};
use self::error::{AddActorError, AddActorErrorReason};

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
    /// Inside of the system, shared (via weak references) with
    /// `ActorSystemRef`s.
    inner: Rc<RefCell<ActorSystemInner>>,
}

impl ActorSystem {
    /// Add a new actor to the system.
    // TODO: keep this in sync with `ActorSystemRef.add_actor`.
    // TODO: replace `'static` lifetime with `'a`.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor<'static> + 'static,
    {
        self.inner.borrow_mut().add_actor(actor, options)
    }

    /// Run the actor system.
    pub fn run<I>(&mut self, initiators: &mut [I]) -> io::Result<()>
        where I: Initiator,
    {
        let system_ref = self.create_ref();
        self.inner.borrow_mut().run(initiators, system_ref)
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: Rc::downgrade(&self.inner),
        }
    }
}

/// A reference to an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemRef {
    /// A non-owning reference to the actor system internals.
    inner: Weak<RefCell<ActorSystemInner>>,
}

impl ActorSystemRef {
    /// Add a new actor to the system.
    ///
    /// See [`ActorSystem.add_actor`].
    ///
    /// [`ActorSystem.add_actor`]: struct.ActorSystem.html#method.add_actor
    // TODO: keep this in sync with `ActorSystemRef.add_actor`.
    // TODO: replace `'static` lifetime with `'a`.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor<'static> + 'static,
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().add_actor(actor, options),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Queue an process to run.
    fn queue_process(&mut self, _id: ProcessId) {
        unimplemented!("ActorSystemRef.queue_process");
    }
}

/// Inside of the `ActorSystem`, to which `ActorSystemRef`s have a reference to.
#[derive(Debug)]
struct ActorSystemInner {
    scheduler: Scheduler,
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
    poll: Poll,
}

impl ActorSystemInner {
    /// Add a new actor to the system.
    // TODO: keep this in sync with `ActorSystemRef.add_actor`.
    // TODO: replace `'static` lifetime with `'a`.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor<'static> + 'static,
    {
        // TODO: return a different error then an `io::Error`.
        let pid = self.pid_gen.next();
        let process = ActorProcess::new(pid, actor, options, &mut self.poll)
            .map_err(|(actor, err)| AddActorError::new(actor, AddActorErrorReason::RegisterFailed(err)))?;
        let actor_ref = process.create_ref();
        let process: ProcessPtr = Box::new(process);
        self.scheduler.add_process(process);
        Ok(actor_ref)
    }

    /// Run the actor system.
    pub fn run<I>(&mut self, _initiators: &mut [I], system_ref: ActorSystemRef) -> io::Result<()>
        where I: Initiator,
    {
        // TODO: return RuntimeError.

        // TODO: register initiatiors with Poll.

        let mut events = Events::new();
        self.poll.poll(&mut events, None)?;

        for event in &mut events {
            let pid = event.id().into();
            self.scheduler.schedule(pid)
                .expect("TODO: handle this error");
        }

        self.scheduler.run(system_ref);
        Ok(())

        // TODO: loop above.
    }
}
