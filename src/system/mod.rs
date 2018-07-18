//! TODO: docs

use std::io;
use std::future::FutureObj;
use std::task::{Executor, SpawnObjError, SpawnErrorKind};
use std::time::Duration;

use mio_st::event::{Events, Evented, Ready};
use mio_st::poll::{Poller, PollOption};
use mio_st::registration::Registration;

use actor::Actor;
use initiator::Initiator;
use process::{ProcessId, ActorProcess, InitiatorProcess, TaskProcess};
use scheduler::{Scheduler, SchedulerRef, Priority};
use util::{Shared, WeakShared};

mod builder;
mod waker;
mod actor_ref;
mod mailbox;

pub mod error;
pub mod options;

pub use self::actor_ref::ActorRef;
pub use self::builder::ActorSystemBuilder;
pub use self::options::{ActorOptions, InitiatorOptions};

// Both used by tests.
pub(crate) use self::mailbox::MailBox;
pub(crate) use self::waker::Waker;

use self::error::{AddActorError, AddActorErrorReason, AddInitiatorError, AddInitiatorErrorReason, RuntimeError, ERR_SYSTEM_SHUTDOWN};

/// The system that runs all actors.
#[derive(Debug)]
pub struct ActorSystem {
    /// Inside of the system, shared (via weak references) with
    /// `ActorSystemRef`s.
    inner: Shared<ActorSystemInner>,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
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
        self.inner.borrow_mut().add_actor(options, actor, system_ref)
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
            inner: self.inner.downgrade(),
        }
    }

    /// Run the actor system.
    pub fn run(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system");

        // Timeout used in the system poller.
        let timeout = self.determine_timeout();
        // Empty set of events, to be filled by the system poller.
        let mut events = Events::new();
        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        // TODO: find a good balance between polling and running processes, the
        // current one is not good.
        loop {
            // Get the scheduled processes.
            self.poll(&mut events, timeout)?;

            // Allow the system to be run without any initiators. In that case
            // we will only handle user space events (e.g. sending messages) and
            // will return after those are all handled.
            // TODO: handle the case where all initiators are removed from the
            // system (due to errors).
            if !self.has_initiators && events.is_empty() {
                debug!("no events, no initiators stopping actor system");
                return Ok(())
            }

            // Schedule all processes with a notification.
            for event in &mut events {
                self.scheduler.schedule(event.id().into());
            }

            // TODO: do something with if no process is run. Maybe return?
            let _ = self.scheduler.run_next(&mut system_ref);
        }
    }

    /// In case of no initiators only user space events are handled and timeout
    /// will be 0ms, otherwise it will be none.
    fn determine_timeout(&self) -> Option<Duration> {
        if self.has_initiators {
            None
        } else {
            Some(Duration::from_millis(0))
        }
    }

    /// Get the set of scheduled processes, replacing it with an empty set.
    ///
    /// This polls the system poller, swaps the scheduled processes in the
    /// scheduler and schedules any processes based on the system poller events.
    ///
    /// # Panics
    ///
    /// Will panic if the actor system inside is already borrowed.
    fn poll(&mut self, events: &mut Events, timeout: Option<Duration>) -> Result<(), RuntimeError> {
        debug!("polling system poller for events");
        self.inner.borrow_mut().poller.poll(events, timeout)
            .map_err(|err| RuntimeError::Poll(err))
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
    inner: WeakShared<ActorSystemInner>,
}

impl ActorSystemRef {
    /// Create a new `ActorSystemRef` that can be used in unit testing.
    ///
    /// # Notes
    ///
    /// All methods will always return a system shutdown error.
    #[cfg(feature = "test")]
    pub fn test_ref() -> ActorSystemRef  {
        ActorSystemRef {
            inner: WeakShared::new(),
        }
    }

    /// Whether or the system is shutdown.
    pub(crate) fn is_shutdown(&self) -> bool {
        match self.inner.upgrade() {
            Some(_) => false,
            None => true,
        }
    }

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
            Some(mut inner) => Ok(inner.borrow_mut().add_actor(options, actor, system_ref)),
            None => Err(AddActorError::new(actor, AddActorErrorReason::SystemShutdown)),
        }
    }

    /// Add an actor that needs to be initialised.
    ///
    /// This is used by the `Initiator`s to register with the system poller with
    /// using same pid.
    pub(crate) fn add_actor_setup<F, A>(&mut self, options: ActorOptions, f: F) -> io::Result<()>
        where F: FnOnce(ProcessId, &mut Poller) -> io::Result<A>,
              A: Actor + 'static,
    {
        let system_ref = self.clone();
        match self.inner.upgrade() {
            Some(mut inner) => inner.borrow_mut().add_actor_setup(options, f, system_ref)
                .map(|_| ()),
            None => Err(AddActorError::new((), AddActorErrorReason::SystemShutdown).into()),
        }
    }

    /// Deregister an `Evented` handle, see `Poll.deregister`.
    pub(crate) fn poller_deregister<E>(&mut self, handle: &mut E) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        match self.inner.upgrade() {
            Some(mut inner) => inner.borrow_mut().poller.deregister(handle),
            None => Err(io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN)),
        }
    }
}

impl Executor for ActorSystemRef {
    fn spawn_obj(&mut self, task: FutureObj<'static, ()>) -> Result<(), SpawnObjError> {
        match self.inner.upgrade() {
            Some(mut inner) => {
                inner.borrow_mut().add_task(task);
                Ok(())
            },
            None => Err(SpawnObjError { kind: SpawnErrorKind::shutdown(), task }),
        }
    }

    fn status(&self) -> Result<(), SpawnErrorKind> {
        match self.inner.upgrade() {
            Some(_) => Ok(()),
            None => Err(SpawnErrorKind::shutdown()),
        }
    }
}

impl Clone for ActorSystemRef {
    fn clone(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: self.inner.clone(),
        }
    }
}

/// Inside of the `ActorSystem`, to which `ActorSystemRef`s have a reference to.
#[derive(Debug)]
struct ActorSystemInner {
    // Scheduler that hold the processes, schedules and runs them.
    //scheduler: Scheduler,
    scheduler_ref: SchedulerRef,
    /// System poller, used for event notifications to support non-blocking I/O.
    poller: Poller,
}

impl ActorSystemInner {
    fn add_actor<A>(&mut self, options: ActorOptions, actor: A, system_ref: ActorSystemRef) -> ActorRef<A>
        where A: Actor + 'static,
    {
        self.add_actor_setup(options, move |_, _| Ok(actor), system_ref)
            .unwrap()
    }

    fn add_actor_setup<F, A>(&mut self, options: ActorOptions, f: F, system_ref: ActorSystemRef) -> io::Result<ActorRef<A>>
        where F: FnOnce(ProcessId, &mut Poller) -> io::Result<A>,
              A: Actor + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.id();
        debug!("adding actor to actor system: pid={}", pid);

        let actor = f(pid, &mut self.poller)?;

        let (mut registration, notifier) = Registration::new();
        self.poller.register(&mut registration, pid.into(), Ready::READABLE, PollOption::Edge)?;

        // Create a new actor process.
        let priority = options.priority;
        let waker = Waker::new(notifier.clone());
        let mailbox = Shared::new(MailBox::new(notifier, system_ref));
        // Create a reference to the actor, to be returned.
        let actor_ref = ActorRef::new(mailbox.downgrade());
        let process = ActorProcess::new(actor, registration, waker, mailbox);

        // Actually add the process.
        process_entry.add(process, priority);
        Ok(actor_ref)
    }

    fn add_initiator<I>(&mut self, mut initiator: I, _options: InitiatorOptions) -> Result<(), AddInitiatorError<I>>
        where I: Initiator + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.id();
        debug!("adding initiator to actor system: pid={}", pid);

        // Initialise the initiator.
        if let Err(err) = initiator.init(&mut self.poller, pid) {
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

    fn add_task(&mut self, task: FutureObj<'static, ()>) {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.id();
        debug!("adding task to actor system: pid={}", pid);

        let (mut registration, notifier) = Registration::new();
        self.poller.register(&mut registration, pid.into(), Ready::READABLE, PollOption::Edge)
            .unwrap(); // Only returns an error in case of double register.

        // Create a new task process.
        let waker = Waker::new(notifier);
        let process = TaskProcess::new(task, registration, waker);

        // Actually add the process.
        // TODO: add an option to the `ActorSystemBuilder` to change the
        // priority.
        process_entry.add(process, Priority::NORMAL);
    }
}
