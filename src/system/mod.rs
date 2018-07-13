//! TODO: docs

use std::io;
use std::cell::RefCell;
use std::collections::HashSet;
use std::future::FutureObj;
use std::rc::{Rc, Weak};
use std::task::{Executor, SpawnObjError, SpawnErrorKind};
use std::time::{Duration, Instant};

use mio_st::event::{Events, Evented};
use mio_st::poll::Poller;

use actor::Actor;
use initiator::Initiator;
use process::{ProcessId, ProcessResult, ActorProcess, InitiatorProcess, TaskProcess};
use scheduler::{Scheduler, Priority, ProcessData, ScheduledProcess};

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
        match self.inner.try_borrow_mut() {
            Ok(mut inner) => inner.add_actor(actor, options, system_ref),
            Err(_) => unreachable!("can't add actor, actor system already borrowed"),
        }
    }

    /// Add a new initiator to the system.
    // TODO: remove `'static` lifetime.
    pub fn add_initiator<I>(&mut self, initiator: I, options: InitiatorOptions) -> Result<(), AddInitiatorError<I>>
        where I: Initiator + 'static,
    {
        match self.inner.try_borrow_mut() {
            Ok(mut inner) => match inner.add_initiator(initiator, options) {
                Ok(()) => {
                    self.has_initiators = true;
                    Ok(())
                },
                err => err,
            },
            Err(_) => unreachable!("can't add initiator, actor system already borrowed"),
        }
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            inner: Rc::downgrade(&self.inner),
        }
    }

    /// Run the actor system.
    pub fn run(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system");

        // Timeout used in the system poller.
        let timeout = self.determine_timeout();
        // Empty set of events, to be filled by the system poller.
        let mut events = Events::new();

        // Empty set of scheduled processes, to be filled by the scheduler.
        let mut scheduled = HashSet::new();
        // Empty process placeholder while we take owner ship, and remove, of a
        // process we need to run.
        let mut empty_process: Box<dyn ScheduledProcess> = Box::new(ProcessData::empty());

        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        loop {
            // Get the scheduled processes.
            scheduled = self.get_scheduled(scheduled, &mut events, timeout)?;

            // Allow the system to be run without any initiators. In that case
            // we will only handle user space events (e.g. sending messages) and
            // will return after those are all handled.
            if !self.has_initiators && scheduled.is_empty() {
                debug!("no events, no initiators stopping actor system");
                return Ok(())
            }

            // Run each process.
            for pid in scheduled.drain() {
                empty_process = self.run_process(pid, empty_process, &mut system_ref);
            }
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
    fn get_scheduled(&mut self, empty: HashSet<ProcessId>, events: &mut Events, timeout: Option<Duration>) -> Result<HashSet<ProcessId>, RuntimeError> {
        debug!("polling system poller for events");
        match self.inner.try_borrow_mut() {
            Ok(mut inner) => match inner.poller.poll(events, timeout) {
                Ok(()) => { /* Continue. */ },
                Err(err) => return Err(RuntimeError::Poll(err)),
            },
            Err(_) => unreachable!("can't poll, actor system already borrowed"),
        }

        // Swap the set of scheduled processes with an empty set.
        let mut scheduled = match self.inner.try_borrow_mut() {
            Ok(mut inner) => inner.scheduler.swap_scheduled(empty),
            Err(_) => unreachable!("can't get scheduled processes, actor system already borrowed"),
        };

        // Schedule any processes that we're notified off.
        for event in events {
            let pid = event.id().into();
            let _ = scheduled.insert(pid);
        }

        Ok(scheduled)
    }

    /// Run a single process.
    ///
    /// It returns the `empty` process.
    ///
    /// # Panics
    ///
    /// Will panic if the actor system inside is already borrowed.
    fn run_process(&mut self, pid: ProcessId, empty: Box<dyn ScheduledProcess>, system_ref: &mut ActorSystemRef) -> Box<dyn ScheduledProcess> {
        let mut process = match self.inner.try_borrow_mut() {
            Ok(mut inner) => match inner.scheduler.swap_process(pid, empty) {
                Ok(process) => process,
                Err(empty) => {
                    debug!("process scheduled, but no longer alive: pid={}", pid);
                    return empty;
                },
            },
            Err(_) => unreachable!("can't swap processes, actor system already borrowed"),
        };

        let start = Instant::now();
        debug!("running process: pid={}", pid);
        let result = process.run(system_ref);
        debug!("finished running process: pid={}, elapsed_time={:?}", pid, start.elapsed());

        match self.inner.try_borrow_mut() {
            Ok(mut inner) => match result {
                ProcessResult::Pending =>
                    inner.scheduler.swap_process(pid, process).unwrap(),
                ProcessResult::Complete =>
                    inner.scheduler.remove_process(pid),
            },
            Err(_) => unreachable!("can't get scheduler, actor system already borrowed"),
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
    /// Create a new `ActorSystemRef` that can be used in unit testing.
    ///
    /// # Notes
    ///
    /// All methods will always return a system shutdown error.
    #[cfg(feature = "test")]
    pub fn test_ref() -> ActorSystemRef  {
        ActorSystemRef {
            inner: Weak::new(),
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
            Some(inner_ref) => match inner_ref.try_borrow_mut() {
                Ok(mut inner) => Ok(inner.add_actor(actor, options, system_ref)),
                Err(_) => unreachable!("can't add actor, actor system already borrowed"),
            },
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
            Some(inner_ref) => match inner_ref.try_borrow_mut() {
                Ok(mut inner) => inner.add_actor_setup(options, f, system_ref),
                Err(_) => unreachable!("can't add actor, actor system already borrowed"),
            },
            None => Err(AddActorError::new((), AddActorErrorReason::SystemShutdown).into()),
        }
    }

    /// Deregister an `Evented` handle, see `Poll.deregister`.
    pub(crate) fn poller_deregister<E>(&mut self, handle: &mut E) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        match self.inner.upgrade() {
            Some(inner_ref) => match inner_ref.try_borrow_mut() {
                Ok(mut inner) => inner.poller.deregister(handle),
                Err(_) => unreachable!("can't deregister with system poller, actor system already borrowed"),
            },
            None => Err(io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN)),
        }
    }

    /// Schedule the process with the provided `pid`.
    ///
    /// If the system is shutdown it will return an error.
    pub(crate) fn schedule(&mut self, pid: ProcessId) -> Result<(), ()> {
        match self.inner.upgrade() {
            Some(inner_ref) => match inner_ref.try_borrow_mut() {
                Ok(mut inner) => {inner.schedule(pid); Ok(()) },
                Err(_) => unreachable!("can't schedule process, actor system already borrowed"),
            },
            None => Err(()),
        }
    }
}

impl Executor for ActorSystemRef {
    fn spawn_obj(&mut self, task: FutureObj<'static, ()>) -> Result<(), SpawnObjError> {
        match self.inner.upgrade() {
            Some(inner_ref) => match inner_ref.try_borrow_mut() {
                Ok(mut inner) => { inner.add_task(task, self.clone()); Ok(()) },
                Err(_) => unreachable!("can't spawn task, actor system already borrowed"),
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
            inner: Weak::clone(&self.inner),
        }
    }
}

/// Inside of the `ActorSystem`, to which `ActorSystemRef`s have a reference to.
#[derive(Debug)]
struct ActorSystemInner {
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// System poller, used for event notifications to support non-blocking I/O.
    poller: Poller,
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
        let waker = Waker::new(pid, system_ref.clone());
        let mailbox = Rc::new(RefCell::new(MailBox::new(pid, system_ref)));
        // Create a reference to the actor, to be returned.
        let actor_ref = ActorRef::new(Rc::downgrade(&mailbox));
        let process = ActorProcess::new(actor, waker, mailbox);


        // Actually add the process.
        process_entry.add(process, priority);
        actor_ref
    }

    fn add_actor_setup<F, A>(&mut self, options: ActorOptions, f: F, system_ref: ActorSystemRef) -> io::Result<()>
        where F: FnOnce(ProcessId, &mut Poller) -> io::Result<A>,
              A: Actor + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler.add_process();
        let pid = process_entry.id();
        debug!("adding actor to actor system: pid={}", pid);

        // Create a new actor process.
        let priority = options.priority;
        let actor = f(pid, &mut self.poller)?;
        let waker = Waker::new(pid, system_ref.clone());
        let mailbox = Rc::new(RefCell::new(MailBox::new(pid, system_ref)));
        let process = ActorProcess::new(actor, waker, mailbox);

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

    fn add_task(&mut self, task: FutureObj<'static, ()>, system_ref: ActorSystemRef) {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler.add_process();
        let pid = process_entry.id();
        debug!("adding task to actor system: pid={}", pid);

        // Create a new task process.
        let waker = Waker::new(pid, system_ref);
        let process = TaskProcess::new(task, waker);

        // Actually add the process.
        // TODO: add an option to the `ActorSystemBuilder` to change the
        // priority.
        process_entry.add(process, Priority::NORMAL);
    }
}
