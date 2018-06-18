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
mod builder;
mod process;
mod scheduler;

pub mod error;
pub mod options;

pub use self::actor_process::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::{ActorOptions, InitiatorOptions};

use self::actor_process::ActorProcess;
use self::scheduler::Scheduler;
use self::process::{ProcessId, ProcessIdGenerator};
use self::error::{AddActorError, AddActorErrorReason, ERR_SYSTEM_SHUTDOWN};

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
    pub fn run<I>(self, initiators: &mut [I]) -> io::Result<()>
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
    pub(crate) fn get_new_pid(&mut self) -> ProcessId {
        // TODO: return `Result` here.
        let r = self.inner.upgrade().unwrap();
        let mut inner = r.borrow_mut();
        inner.pid_gen.next()
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
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
    /// System poller, used for event notifications to support non-block I/O.
    poll: Poll,
}

impl ActorSystemInner {
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        let pid = self.pid_gen.next();
        self.add_actor_pid(actor, options, pid)
    }

    pub fn add_actor_pid<A>(&mut self, actor: A, options: ActorOptions, pid: ProcessId) -> Result<ActorRef<A>, AddActorError<A>>
        where A: Actor + 'static,
    {
        debug!("adding actor with pid={} to actor system", pid);
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

    pub fn run<I>(&mut self, initiators: &mut [I], mut system_ref: ActorSystemRef) -> io::Result<()>
        where I: Initiator,
    {
        // TODO: return RuntimeError.

        // Timeout for polling. None if there are any initiators, or 0 ms in
        // case of no initiators so only user space events are handled and
        // stopped otherwise.
        let timeout = if initiators.is_empty() {
            debug!("actor system running without initiators, thus 0ms timeout");
            Some(Duration::from_millis(0))
        } else {
            debug!("actor system running with initiators, thus with no timeout");
            None
        };

        let mut events = Events::new();

        loop {
            self.poll.poll(&mut events, timeout)?;

            // Allow the system to be run without any initiators. In that case
            // we will only handle user space events (e.g. sending messages) and
            // will return after those are all handled.
            if initiators.is_empty() && events.is_empty() {
                return Ok(())
            }

            // Schedule any processes that we're notified off.
            for event in &mut events {
                let pid = event.id().into();
                // TODO: handle this error
                let _ = self.scheduler.schedule(pid);
            }

            // Run all scheduled processes.
            self.scheduler.run(&mut system_ref);
        }
    }
}
