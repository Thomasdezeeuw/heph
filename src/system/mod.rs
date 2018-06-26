//! TODO: docs

use std::io;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use mio_st::event::{Events, Evented};
use mio_st::poll::Poll;

use actor::Actor;
use initiator::Initiator;
use process::{ProcessId, ActorProcess, InitiatorProcess};
use scheduler::{Scheduler, Priority};

mod builder;

pub mod error;
pub mod options;

pub use process::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::{ActorOptions, InitiatorOptions};

use self::error::{AddActorError, AddActorErrorReason, AddInitiatorError, AddInitiatorErrorReason, RuntimeError, ERR_SYSTEM_SHUTDOWN};

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
    /// Inside of the system, shared (via weak references) with
    /// `ActorSystemRef`s.
    inner: Rc<RefCell<ActorSystemInner>>,
    /// Whether or not the system has initiators, this is used to allow the
    /// system to run without them. Otherwise we would poll with no timeout,
    /// waiting for ever.
    has_initiators: bool,
}

impl ActorSystem {
    /// Add a new actor to the system.
    // TODO: remove `'static` lifetime.
    pub fn add_actor<A>(&mut self, actor: A, options: ActorOptions) -> ActorRef<A>
        where A: Actor + 'static,
    {
        let system_ref = self.create_ref();
        self.inner.borrow_mut().add_actor(actor, options, system_ref)
    }

    /// Add a new initiator to the system.
    // TODO: remove `'static` lifetime.
    pub fn add_initiator<I>(&mut self, initiator: I, options: InitiatorOptions) -> Result<(), AddInitiatorError<I>>
        where I: Initiator + 'static,
    {
        match self.inner.borrow_mut().add_initiator(initiator, options) {
            Ok(()) => {
                self.has_initiators = true;
                Ok(())
            },
            err => err,
        }
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: Rc::downgrade(&self.inner),
        }
    }

    /// Run the actor system.
    pub fn run(self) -> Result<(), RuntimeError> {
        debug!("running actor system");

        // In case of no initiators only user space events are handled and the
        // system is stopped otherwise.
        let timeout = if self.has_initiators {
            None
        } else {
            Some(Duration::from_millis(0))
        };

        let mut inner = self.inner.borrow_mut();

        let mut events = Events::new();
        let mut system_ref = self.create_ref();
        loop {
            debug!("polling system poll for events");
            inner.poll.poll(&mut events, timeout)
                .map_err(RuntimeError::Poll)?;

            // Allow the system to be run without any initiators. In that case
            // we will only handle user space events (e.g. sending messages) and
            // will return after those are all handled.
            if !self.has_initiators && events.is_empty() && inner.scheduler.scheduled() == 0 {
                debug!("no events, no initiators stopping actor system");
                return Ok(())
            }

            // Schedule any processes that we're notified off.
            for event in &mut events {
                let pid = event.id().into();
                inner.scheduler.schedule(pid);
            }

            // Run all scheduled processes.
            inner.scheduler.run(&mut system_ref);
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
        let system_ref = self.clone();
        match self.inner.upgrade() {
            Some(r) => Ok(r.borrow_mut().add_actor(actor, options, system_ref)),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Add an actor that needs to be initialised.
    ///
    /// This is used by the `Initiator`s to register with poll with the same
    /// pid.
    pub(crate) fn add_actor_pid<F, A>(&mut self, options: ActorOptions, f: F) -> io::Result<()>
        where F: FnOnce(ProcessId, &mut Poll) -> io::Result<A>,
              A: Actor + 'static,
    {
        let system_ref = self.clone();
        match self.inner.upgrade() {
            Some(r) => r.borrow_mut().add_actor_pid(options, f, system_ref),
            None => Err(AddActorError::new((), AddActorErrorReason::SystemShutdown).into()),
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

    /// Schedule the process with the provided `pid`.
    ///
    /// If the system is shutdown it will return an error.
    pub(crate) fn schedule(&mut self, pid: ProcessId) -> Result<(), ()> {
        match self.inner.upgrade() {
            Some(r) => {
                r.borrow_mut().schedule(pid);
                Ok(())
            },
            None => Err(()),
        }
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
    /// System poller, used for event notifications to support non-block I/O.
    poll: Poll,
}

impl ActorSystemInner {
    fn add_actor<A>(&mut self, actor: A, options: ActorOptions, system_ref: ActorSystemRef) -> ActorRef<A>
        where A: Actor + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler.add_process();
        let pid = process_entry.id();
        debug!("adding actor to actor system: pid={}", pid);

        // Create a new actor process.
        let priority = options.priority;
        let process = ActorProcess::new(pid, actor, options, system_ref);

        // Create a reference to the actor, to be returned.
        let actor_ref = process.create_ref();

        // Actually add the process.
        process_entry.add(process, priority);
        actor_ref
    }

    fn add_actor_pid<F, A>(&mut self, options: ActorOptions, f: F, system_ref: ActorSystemRef) -> io::Result<()>
        where F: FnOnce(ProcessId, &mut Poll) -> io::Result<A>,
              A: Actor + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler.add_process();
        let pid = process_entry.id();
        debug!("adding actor to actor system: pid={}", pid);

        // Create a new actor process.
        let priority = options.priority;
        let actor = f(pid, &mut self.poll)?;
        let process = ActorProcess::new(pid, actor, options, system_ref);

        // Actually add the process.
        process_entry.add(process, priority);
        Ok(())
    }

    fn schedule(&mut self, pid: ProcessId) {
        self.scheduler.schedule(pid);
    }

    fn add_initiator<I>(&mut self, mut initiator: I, _options: InitiatorOptions) -> Result<(), AddInitiatorError<I>>
        where I: Initiator + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler.add_process();
        let pid = process_entry.id();
        debug!("adding initiator to actor system: pid={}", pid);

        // Initialise the initiator.
        if let Err(err) = initiator.init(&mut self.poll, pid) {
            return Err(AddInitiatorError {
                initiator,
                reason: AddInitiatorErrorReason::InitFailed(err),
            });
        }

        // Create a new initiator process.
        let process = InitiatorProcess::new(initiator);

        // Actually add the process.
        // Initiators will always have a low priority this way requests in
        // progress are first handled before new requests are accepted and
        // possibly overload the system.
        process_entry.add(process, Priority::LOW);
        Ok(())
    }
}
