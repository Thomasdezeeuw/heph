//! TODO: docs

use std::io;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use futures_core::task::{Context, LocalMap, Waker};
use mio_st::event::{Events, Evented, EventedId, Ready};
use mio_st::poll::{Poll, PollOpt};

use actor::Actor;
use initiator::Initiator;

mod actor_process;
mod initiator_process;
mod builder;
mod process;
mod scheduler;

pub mod error;
pub mod options;

pub use self::actor_process::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::{ActorOptions, InitiatorOptions};

use self::actor_process::ActorProcess;
use self::error::{AddActorError, AddActorErrorReason, AddInitiatorError, AddInitiatorErrorReason, RuntimeError, ERR_SYSTEM_SHUTDOWN};
use self::initiator_process::InitiatorProcess;
use self::process::{ProcessId, ProcessIdGenerator};
use self::scheduler::Scheduler;

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
    /// Inside of the system, shared (via weak references) with
    /// `ActorSystemRef`s.
    inner: Rc<RefCell<ActorSystemInner>>,
}

impl ActorSystem {
    /// Add a new actor to the system.
    // TODO: remove `'static` lifetime.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A>, AddActorError<A>>
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
    pub fn run(self) -> Result<(), RuntimeError> {
        let mut system_ref = self.create_ref();
        debug!("running actor system");

        loop {
            debug!("polling system poll for events");
            let n_events = self.inner.borrow_mut().poll()
                .map_err(|err| RuntimeError::Poll(err))?;

            // Allow the system to be run without any initiators. In that case
            // we will only handle user space events (e.g. sending messages) and
            // will return after those are all handled.
            if !self.inner.borrow().has_initiators && n_events == 0 {
                debug!("no events, no initiators stopping actor system");
                return Ok(())
            }

            // Run all scheduled processes.
            self.inner.borrow_mut().scheduler.run(&mut system_ref);
        }
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
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().add_actor(actor, options),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Same as `add_actor` but used the provided `pid` as process id.
    ///
    /// Th
    pub(crate) fn add_actor_pid<A>(&mut self, actor: A, options: ActorOptions, pid: ProcessId) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().add_actor_pid(actor, options, pid),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Get a new process id also to be used as `EventedId`.
    ///
    /// For example used in `add_actor_pid` or in registering a TCP listener.
    pub(crate) fn next_pid(&mut self) -> ProcessId {
        // TODO: return `Result` here.
        let r = self.inner.upgrade().unwrap();
        let mut inner = r.borrow_mut();
        inner.next_pid()
    }

    /// Register an `Evented` handle, see `Poll.register`.
    pub(crate) fn poll_register<E>(&mut self, handle: &mut E, id: EventedId, interests: Ready, opt: PollOpt) -> io::Result<()>
        where E: Evented + ?Sized
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().poll.register(handle, id, interests, opt),
            None => Err(io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN)),
        }
    }

    /// Deregister an `Evented` handle, see `Poll.deregister`.
    pub(crate) fn poll_deregister<E>(&mut self, handle: &mut E) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().poll.deregister(handle),
            None => Err(io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN)),
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
    /// Whether or not the system has initiators, this is used to allow the
    /// system to run without them. Otherwise we would poll with no timeout,
    /// waiting for ever.
    has_initiators: bool,
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
    /// System poller, used for event notifications to support non-block I/O.
    poll: Poll,
}

impl ActorSystemInner {
    fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        let pid = self.pid_gen.next();
        self.add_actor_pid(actor, options, pid)
    }

    fn add_actor_pid<A>(&mut self, actor: A, options: ActorOptions, pid: ProcessId) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        debug!("adding actor to actor system: pid={}", pid);
        // Create a new actor process.
        let process = ActorProcess::new(pid, actor, options, &mut self.poll)
            .map_err(|(actor, err)| AddActorError::new(actor, AddActorErrorReason::RegisterFailed(err)))?;

        // Create a reference to the actor, to be returned.
        let actor_ref = process.create_ref();

        // Add the process to the scheduler, it will be consider inactive.
        let process = Box::new(process);
        self.scheduler.add_process(process);

        Ok(actor_ref)
    }

    /// Get the next unique `ProcessId`.
    pub(crate) fn next_pid(&mut self) -> ProcessId {
        self.pid_gen.next()
    }

    /// Poll the system poll and schedule the notified processes, returns the
    /// number of processes scheduled.
    fn poll(&mut self) -> io::Result<usize> {
        let mut events = Events::new();

        // In case of no initiators only user space events are handled and the
        // system is stopped otherwise.
        let timeout = if self.has_initiators {
            None
        } else {
            Some(Duration::from_millis(0))
        };

        self.poll.poll(&mut events, timeout)?;

        // Schedule any processes that we're notified off.
        let n_scheduled = (&mut events).fold(0, |acc, event| {
            let pid = event.id().into();
            // TODO: handle this error
            if let Err(_) = self.scheduler.schedule(pid) {
                error!("unable to schedule process: pid={}", pid);
                acc
            } else {
                acc + 1
            }
        });

        Ok(n_scheduled)
    }
}
