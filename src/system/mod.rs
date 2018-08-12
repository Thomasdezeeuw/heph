//! Module with the actor system and related types.
//!
//! The module has two types and a submodule:
//!
//! - [`ActorSystem`]: is the actor system, used to run all actors.
//! - [`ActorSystemRef`]: is a reference to the actor system, used to get access
//!   to the actor system's internal, often via the [`ActorContext`], e.g. see
//!   [`TcpStream.connect`].
//!
//! See [`ActorSystem`] for documentation on how to run an actor system. For
//! more examples see the examples directory in the source code.
//!
//! [`ActorSystem`]: struct.ActorSystem.html
//! [`ActorSystemRef`]: struct.ActorSystemRef.html
//! [`ActorContext`]: ../actor/struct.ActorContext.html
//! [`TcpStream.connect`]: ../net/struct.TcpStream.html#method.connect

use std::{fmt, io};
use std::future::FutureObj;
use std::task::{Executor, SpawnObjError, SpawnErrorKind};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use num_cpus;
use crossbeam_channel::{self as channel, Receiver, Sender};
use log::{debug, trace, log};
use mio_st::event::{Evented, Events, EventedId, Ready};
use mio_st::poll::{Poller, PollOption};

use crate::actor::{Actor, ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::error::RuntimeError;
use crate::initiator::Initiator;
use crate::mailbox::MailBox;
use crate::process::{ProcessId, ActorProcess, InitiatorProcess, TaskProcess};
use crate::scheduler::{Scheduler, SchedulerRef, Priority};
use crate::util::Shared;
use crate::waker::new_waker;

pub mod options;

pub use self::options::{ActorOptions, InitiatorOptions};

/// The system that runs all actors.
///
/// This type implements a builder pattern to build and run an actor system. It
/// has two generic parameters `I` and `S`, the types of the initiator(s) and a
/// setup function. Both types are optional, which is represented by `!` (the
/// never type).
///
/// ## Usage
///
/// Building an actor system starts with calling [`new`], this will create a new
/// system without initiators or a setup function.
///
/// An optional [`Initiator`] can be added using the [`with_initiator`] method,
/// this will change the generic parameter from `!` to `I` (the type of the
/// `Initiator`). This method can only be called once, to add more `Initiator`s
/// the [`add_initiator`] method can be used.
///
/// An optional setup function can also be added. This setup function will be
/// called on each thread the actor system starts, and thus has to be `Clone`
/// and `Send`. This can be done by calling [`with_setup`]. This will changes
/// the `S` parameter from `!` to an actual type. Only a single setup function
/// can be used.
///
/// Now both generic parameter are optionally defined we also have some more
/// configuration options. One such option is the number of threads the system
/// uses, this can configured with the [`num_threads`] method, or
/// [`use_all_cores`].
///
/// Finally after setting all the configuration options the system can be
/// [`run`]. This will spawn a number of threads to actually run the system.
///
/// [`new`]: #method.new
/// [`Initiator`]: ../initiator/trait.Initiator.html
/// [`with_initiator`]: #method.with_initiator
/// [`add_initiator`]: #method.add_initiator
/// [`with_setup`]: #method.with_setup
/// [`num_threads`]: #method.num_threads
/// [`use_all_cores`]: #method.use_all_cores
/// [`run`]: #method.run
///
/// ## Examples
///
/// This simple Hello World example show how to run an `ActorSystem` and add how
/// start a single actor on each thread. This should print "Hello World" twice.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
///
/// use heph::actor::{ActorContext, actor_factory};
/// use heph::system::{ActorSystem, ActorSystemRef, ActorOptions};
///
/// /// Our actor that greets people.
/// async fn greeter_actor(mut ctx: ActorContext<&'static str>, message: &'static str) -> Result<(), !> {
///     let name = await!(ctx.receive());
///     println!("{} {}", message, name);
///     Ok(())
/// }
///
/// // This setup function will on run on each created thread. In the case of
/// // this example we create 2 threads (see `main`).
/// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
///     // Add the actor to the system.
///     let new_actor = actor_factory(greeter_actor);
///     let mut actor_ref = system_ref.add_actor(new_actor, "Hello", ActorOptions::default());
///
///     // Send a message to the actor.
///     actor_ref.send("World")
///         .expect("unable to send message");
///
///     Ok(())
/// }
///
/// fn main() {
///     // Build a new `ActorSystem`.
///     ActorSystem::new()
///         // Start two worker threads.
///         .num_threads(2)
///         // On each worker thread run our setup function.
///         .with_setup(setup)
///         // And run the system.
///         .run()
///         .expect("unable to run actor system");
/// }
/// ```
pub struct ActorSystem<I = !, S = !> {
    /// Number of threads to create.
    threads: usize,
    /// The initiators to be added to the system.
    initiators: Vec<(I, InitiatorOptions)>,
    /// Optional setup function.
    setup: Option<S>,
}

impl ActorSystem {
    /// Create a new actor system with the default configuration.
    pub fn new() -> ActorSystem<!, !> {
        ActorSystem {
            threads: 1,
            initiators: Vec::new(),
            setup: None,
        }
    }
}

impl<I> ActorSystem<I, !> {
    /// Add a setup function.
    ///
    /// This function will be run on each thread the actor system creates. Only
    /// a single setup function can be added to the actor system.
    pub fn with_setup<S>(self, setup: S) -> ActorSystem<I, S>
        where S: Fn(ActorSystemRef) -> io::Result<()> + Send + Clone + 'static,
    {
        ActorSystem {
            threads: self.threads,
            initiators: self.initiators,
            setup: Some(setup),
        }
    }
}

impl<S> ActorSystem<!, S> {
    /// Add a new initiator to the system, setting the type in the process.
    ///
    /// To add more initiators the [`add_initiator`] method can be used.
    ///
    /// [`add_initiator`]: #method.add_initiator
    pub fn with_initiator<I>(self, initiator: I, options: InitiatorOptions) -> ActorSystem<I, S>
        where I: Initiator + 'static,
    {
        ActorSystem {
            threads: self.threads,
            initiators: vec![(initiator, options)],
            setup: self.setup,
        }
    }
}

impl<I, S> ActorSystem<I, S> {
    /// Set the number of threads used, defaults to `1`.
    ///
    /// Most applications would want to use [`use_all_cores`] which sets the
    /// number of threads equal to the number of cpu cores.
    ///
    /// [`use_all_cores`]: #method.use_all_cores
    pub fn num_threads(mut self, n: usize) -> Self {
        self.threads = n;
        self
    }

    /// Set the number of threads equal to the number of cpu cores.
    ///
    /// See [`num_threads`].
    ///
    /// [`num_threads`]: #method.num_threads
    pub fn use_all_cores(self) -> Self {
        self.num_threads(num_cpus::get())
    }

    /// Add another initiator to the system.
    ///
    /// First [`with_initiator`] must be called.
    ///
    /// [`with_initiator`]: #method.with_initiator
    pub fn add_initiator(mut self, initiator: I, options: InitiatorOptions) -> Self {
        self.initiators.push((initiator, options));
        self
    }
}

impl<I, S> ActorSystem<I, S>
    where S: SetupFn  + Send + Clone + 'static,
          I: Initiator + 'static,
{
    /// Run the system.
    ///
    /// This will spawn a number of threads (see [`num_threads`]) to run the
    /// system.
    ///
    /// [`num_threads`]: #method.num_threads
    pub fn run(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system: threads={}", self.threads);

        let handles = self.start_threads()?;

        // TODO: handle errors better. Currently as long as the first thread
        // keeps running and all other threads crash we'll keep running. Not an
        // ideal situation.
        for handle in handles {
            handle.join()
                .map_err(|err| {
                    let msg = match err.downcast_ref::<&'static str>() {
                        Some(s) => (*s).to_owned(),
                        None => match err.downcast_ref::<String>() {
                            Some(s) => s.clone(),
                            None => "unkown panic message".to_owned(),
                        },
                    };
                    RuntimeError::Panic(msg)
                })
                .and_then(|res| res)?;
        }

        Ok(())
    }

    /// Start the threads for the actor system.
    fn start_threads(&mut self) -> Result<Vec<JoinHandle<Result<(), RuntimeError>>>, RuntimeError> {
        (0..self.threads).map(|_| {
            // Clone the setup and the initiators, using the special clone
            // method.
            let setup = self.setup.clone();
            let initiators = self.initiators.iter().map(|(initiator, options)| {
                initiator.clone_threaded()
                    .map(|i| (i, options.clone()))
                    .map_err(RuntimeError::Initiator)
            })
            .collect::<Result<_, RuntimeError>>()?;

            // Spawn the thread.
            Ok(thread::spawn(move || run_system(initiators, setup)))
        })
        .collect()
    }
}

impl<I, S> fmt::Debug for ActorSystem<I, S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorSystem")
            .field("threads", &self.threads)
            .field("setup", if self.setup.is_some() { &"Some" } else { &"None" })
            .field("initiators (length)", &self.initiators.len())
            .finish()
    }
}

/// A hack to allow us to call `ActorSystem<I, !>.run`.
mod hack {
    use std::io;

    use super::ActorSystemRef;

    pub trait SetupFn {
        fn setup(&self, system_ref: ActorSystemRef) -> io::Result<()>;
    }

    impl<T> SetupFn for T
        where T: Fn(ActorSystemRef) -> io::Result<()>
    {
        fn setup(&self, system_ref: ActorSystemRef) -> io::Result<()> {
            (self)(system_ref)
        }
    }

    impl SetupFn for ! {
        fn setup(&self, _: ActorSystemRef) -> io::Result<()> {
            *self
        }
    }
}

use self::hack::SetupFn;

/// Run the actor system, with the provided `initiators` and optional `setup`
/// function.
///
/// This is the entry point for the worker threads.
fn run_system<I, S>(initiators: Vec<(I, InitiatorOptions)>,  setup: Option<S>) -> Result<(), RuntimeError>
    where S: SetupFn,
          I: Initiator + 'static,
{
    let mut actor_system = RunningActorSystem::new()?;

    // Add all initiators to the actor system.
    for (initiator, options) in initiators {
        actor_system.add_initiator(initiator, options)?;
    }

    // Run optional setup.
    if let Some(setup) = setup {
        let system_ref = actor_system.create_ref();
        setup.setup(system_ref).map_err(RuntimeError::Setup)?;
    }

    // All setup is done, so we're ready to run the event loop.
    actor_system.run_event_loop()
}

/// The system that runs all processes.
#[derive(Debug)]
struct RunningActorSystem {
    /// Inside of the system, shared with zero or more `ActorSystemRef`s.
    internal: Shared<ActorSystemInternal>,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// Whether or not the system has initiators.
    // FIXME: This is currently required mostly for tests and example 1. Try to
    // remove it.
    has_initiators: bool,
    /// Receiving side of the channel for `Waker` notifications.
    waker_notifications: Receiver<ProcessId>,
}

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new() -> Result<RunningActorSystem, RuntimeError> {
        // Channel used in the `Waker` implementation.
        let (waker_send, waker_recv) = channel::unbounded();
        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();
        // System poller for event notifications.
        let poller = Poller::new().map_err(RuntimeError::Poll)?;

        // Internals of the running actor system.
        let internal = ActorSystemInternal::new(scheduler_ref, poller, waker_send);

        // The actor system we're going to run.
        Ok(RunningActorSystem {
            internal: Shared::new(internal),
            scheduler,
            has_initiators: false,
            waker_notifications: waker_recv,
        })
    }

    /// Add an `Initiator` to the system.
    pub fn add_initiator<I>(&mut self, mut initiator: I, _options: InitiatorOptions) -> Result<(), RuntimeError>
        where I: Initiator + 'static,
    {
        // Make sure we call `Poller.poll` without a timeout.
        self.has_initiators = true;

        let ActorSystemInternal {
            ref mut scheduler_ref,
            ref mut poller,
            ..
        } = *self.internal.borrow_mut();

        // Setup adding a new process to the scheduler.
        let process_entry = scheduler_ref.add_process();
        let pid = process_entry.id();
        trace!("adding initiator to scheduler: pid={}", pid);

        // Initialise the initiator.
        trace!("initialising initiator: pid={}", pid);
        initiator.init(poller, pid).map_err(RuntimeError::Initiator)?;

        // Create a new initiator process.
        let process = InitiatorProcess::new(initiator);

        // Add the process to the scheduler.
        // Initiators will always have a low priority this way requests in
        // progress are first handled before new requests are accepted and
        // possibly overload the system.
        process_entry.add(process, Priority::LOW);
        Ok(())
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            internal: self.internal.clone(),
        }
    }

    /// Run the actor system's event loop.
    pub fn run_event_loop(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system's event loop");

        // Empty set of events, to be filled by the system poller.
        let mut events = Events::new();
        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        // TODO: find a good balance between polling, polling user space events
        // only and running processes, the current one is not good. It leans too
        // much to polling.
        loop {
            self.schedule_processes(&mut events)?;

            if !self.scheduler.run_process(&mut system_ref) && events.is_empty() {
                debug!("no events, no processes to run, stopping actor system");
                return Ok(())
            }
        }
    }

    /// Schedule processes.
    ///
    /// This polls the system poller and the waker notifications and schedules
    /// the processes notified.
    fn schedule_processes(&mut self, events: &mut Events) -> Result<(), RuntimeError> {
        trace!("polling system poller for events");
        let timeout = self.determine_timeout();
        self.internal.borrow_mut().poller.poll(events, timeout)
            .map_err(RuntimeError::Poll)?;

        // Schedule all processes with a notification.
        for event in events {
            self.scheduler.schedule(event.id().into());
        }

        trace!("receiving waker events");
        while let Some(pid) = self.waker_notifications.try_recv() {
            self.scheduler.schedule(pid);
        }

        Ok(())
    }

    /// Determine the timeout used in the system poller.
    fn determine_timeout(&self) -> Option<Duration> {
        if !self.has_initiators || self.scheduler.process_ready() {
            Some(Duration::from_millis(0))
        } else {
            None
        }
    }
}

/// A reference to an [`ActorSystem`].
///
/// This reference refers to the thread-local actor system, and thus can't be
/// shared across thread bounds. To share this reference (within the same
/// thread) it can be cloned.
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Clone, Debug)]
pub struct ActorSystemRef {
    /// A shared reference to the actor system's internals.
    internal: Shared<ActorSystemInternal>,
}

impl ActorSystemRef {
    /// Add a new actor to the system.
    pub fn add_actor<N, I, A>(&mut self, new_actor: N, item: I, options: ActorOptions) -> LocalActorRef<N::Message>
        where N: NewActor<Item = I, Actor = A>,
              A: Actor + 'static,
    {
        let system_ref = self.clone();
        self.internal.borrow_mut().add_actor(options, new_actor, item, system_ref)
    }

    /// Add an actor that needs to be initialised.
    ///
    /// This is used by the `Initiator`s to register with the system poller with
    /// using same pid.
    pub(crate) fn add_actor_setup<F, A, M>(&mut self, options: ActorOptions, f: F) -> io::Result<()>
        where F: FnOnce(ActorContext<M>, ProcessId, &mut Poller) -> io::Result<A>,
              A: Actor + 'static,
    {
        let system_ref = self.clone();
        self.internal.borrow_mut().add_actor_setup(options, f, system_ref)
            .map(|_| ())
    }

    /// Register an `Evented` handle, see `Poll.register`.
    pub(crate) fn poller_register<E>(&mut self, handle: &mut E, id: EventedId, interests: Ready, opt: PollOption) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        self.internal.borrow_mut().poller.register(handle, id, interests, opt)
    }

    /// Deregister an `Evented` handle, see `Poll.deregister`.
    pub(crate) fn poller_deregister<E>(&mut self, handle: &mut E) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        self.internal.borrow_mut().poller.deregister(handle)
    }

    /// Get a clone of the sending end of the notification channel.
    pub(crate) fn get_notification_sender(&mut self) -> Sender<ProcessId> {
        self.internal.borrow_mut().waker_notifications.clone()
    }

    /// Add a deadline to the system poller.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.internal.borrow_mut().poller.add_deadline(pid.into(), deadline)
            .unwrap();
    }

    pub(crate) fn notify(&mut self, pid: ProcessId) {
        self.internal.borrow_mut().poller.notify(pid.into(), Ready::READABLE)
            .unwrap();
    }
}

/// This is not a part of the stable API, but an implementation detail. Use the
/// `Executor` on the `task::Context` instead.
#[doc(hidden)]
impl Executor for ActorSystemRef {
    fn spawn_obj(&mut self, task: FutureObj<'static, ()>) -> Result<(), SpawnObjError> {
        self.internal.borrow_mut().add_task(task);
        Ok(())
    }

    fn status(&self) -> Result<(), SpawnErrorKind> {
        Ok(())
    }
}

/// Internals of the `RunningActorSystem`, to which `ActorSystemRef`s have a
/// reference.
#[derive(Debug)]
pub(crate) struct ActorSystemInternal {
    /// A reference to the scheduler to add new processes to.
    scheduler_ref: SchedulerRef,
    /// System poller, used for event notifications to support non-blocking I/O.
    poller: Poller,
    /// Sending side of the channel for `Waker` notifications.
    waker_notifications: Sender<ProcessId>,
}

impl ActorSystemInternal {
    /// Create new actor system internals.
    pub fn new(scheduler_ref: SchedulerRef, poller: Poller, waker_notifications: Sender<ProcessId>) -> ActorSystemInternal {
        ActorSystemInternal {
            scheduler_ref,
            poller,
            waker_notifications,
        }
    }

    pub fn add_actor<N, I, A>(&mut self, options: ActorOptions, mut new_actor: N, item: I, system_ref: ActorSystemRef) -> LocalActorRef<N::Message>
        where N: NewActor<Item = I, Actor = A>,
              A: Actor + 'static,
    {
        self.add_actor_setup(options, move |ctx, _, _| Ok(new_actor.new(ctx, item)), system_ref)
            .unwrap() // Safe because the function doesn't return an error.
    }

    pub fn add_actor_setup<F, A, M>(&mut self, options: ActorOptions, f: F, system_ref: ActorSystemRef) -> io::Result<LocalActorRef<M>>
        where F: FnOnce(ActorContext<M>, ProcessId, &mut Poller) -> io::Result<A>,
              A: Actor + 'static,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.id();
        debug!("adding actor to actor system: pid={}", pid);

        // Create our waker, mailbox and actor reference.
        let waker = new_waker(pid, self.waker_notifications.clone());
        let mailbox = Shared::new(MailBox::new(pid, system_ref.clone()));
        let actor_ref = LocalActorRef::new(mailbox.downgrade());

        // Create the actor context and create an actor with it.
        let ctx = ActorContext::new(pid, system_ref, mailbox);
        let actor = f(ctx, pid, &mut self.poller)?;

        // Create an actor process and add finally add it to the scheduler.
        let process = ActorProcess::new(actor, waker);
        process_entry.add(process, options.priority);
        Ok(actor_ref)
    }

    pub fn add_task(&mut self, task: FutureObj<'static, ()>) {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.id();
        debug!("adding task to actor system: pid={}", pid);

        // Create a new task process.
        let waker = new_waker(pid, self.waker_notifications.clone());
        let process = TaskProcess::new(task, waker);

        // Actually add the process.
        process_entry.add(process, Priority::NORMAL);
    }
}
