//! TODO: docs

use std::io;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use futures_core::task::{Context, LocalMap, Waker};
use mio_st::event::Events;
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
use self::scheduler::Scheduler;
use self::process::ProcessIdGenerator;
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
    // TODO: remove `'static` lifetime.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor + 'static,
    {
        self.inner.borrow_mut().add_actor(actor, options)
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: Rc::downgrade(&self.inner),
        }
    }

    /// Run the actor system.
    ///
    /// The provided `initiators` are optional, if no initiators are required
    /// the [`NoInitiator`] helper can be used.
    ///
    /// [`NoInitiator`]: ../initiator/struct.NoInitiator.html
    ///
    /// # Examples
    ///
    /// Running without initiators.
    ///
    /// ```
    /// use actor::system::ActorSystemBuilder;
    /// use actor::initiator::NoInitiator;
    ///
    /// let mut actor_system = ActorSystemBuilder::default().build()
    ///     .expect("failed to build actor system");
    ///
    /// // Add actors etc.
    ///
    /// // TODO: enable this.
    /// //actor_system.run::<NoInitiator>(&mut [])
    /// //    .expect("failed to run actor system");
    /// ```
    pub fn run<I>(&mut self, initiators: &mut [I]) -> io::Result<()>
        where I: Initiator,
    {
        let system_ref = self.create_ref();
        self.inner.borrow_mut().run(initiators, system_ref)
    }
}

/// A reference to an [`ActorSystem`].
///
/// This reference can be shared by cloning it, a very cheap operation, just
/// like [`ActorRef`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
/// [`ActorRef`]: struct.ActorRef.html
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
    // TODO: remove `'static` lifetime,
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor + 'static,
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().add_actor(actor, options),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Create a new futures' execution `Context`.
    pub(crate) fn create_context<'a>(&'a mut self, map: &'a mut LocalMap, waker: &'a Waker) -> Context<'a> {
        // TODO: add executor.
        Context::without_spawn(map, waker)
    }
}

impl Clone for ActorSystemRef {
    fn clone(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: Weak::clone(&self.inner),
        }
    }
}

/// Inside of the `ActorSystem`, to which `ActorSystemRef`s have a reference to.
#[derive(Debug)]
struct ActorSystemInner {
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
    /// System poller, used for event notifications to support non-block I/O.
    poll: Poll,
}

impl ActorSystemInner {
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A::Message>, AddActorError<A>>
        where A: Actor + 'static,
    {
        // Create a new actor process.
        let pid = self.pid_gen.next();
        let process = ActorProcess::new(pid, actor, options, &mut self.poll)
            .map_err(|(actor, err)| AddActorError::new(actor, AddActorErrorReason::RegisterFailed(err)))?;

        // Create a reference to the actor, to be returned.
        let actor_ref = process.create_ref();

        // Add the process to the scheduler, it will be consider inactive.
        let process = Box::new(process);
        self.scheduler.add_process(process);

        Ok(actor_ref)
    }

    pub fn run<I>(&mut self, _initiators: &mut [I], system_ref: ActorSystemRef) -> io::Result<()>
        where I: Initiator,
    {
        // TODO: return RuntimeError.

        // TODO: register initiators with Poll.

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
