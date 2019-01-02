//! Module with the actor system and related types.
//!
//! The module has two types and a submodule:
//!
//! - [`ActorSystem`]: is the actor system, used to run all actors.
//! - [`ActorSystemRef`]: is a reference to the actor system, used to get access
//!   to the actor system's internals, often via the [`ActorContext`], e.g. see
//!   [`TcpStream.connect`].
//!
//! See [`ActorSystem`] for documentation on how to run an actor system. For
//! more examples see the examples directory in the source code.
//!
//! [`ActorSystem`]: struct.ActorSystem.html
//! [`ActorSystemRef`]: struct.ActorSystemRef.html
//! [`ActorContext`]: ../actor/struct.ActorContext.html
//! [`TcpStream.connect`]: ../net/struct.TcpStream.html#method.connect
//!
//! # Actor Registry
//!
//! The Actor Registry is a fairly simple concept, it maps [`NewActor`]s to
//! [`LocalActorRef`]s.
//!
//! First an actor must be registered with the Actor Registry. This is done by
//! setting the [register] option to `true` in the [`ActorOptions`] passed to
//! [`spawn`] or an [`Initiator`] that spawns actors, e.g. [`TcpListener`]. Note
//! that an actor registration must be unique **per actor type**, that is type
//! in the Rust's type system. It is not possible to register two actors with
//! the same type.
//!
//! After the actor is registered it can be looked up. This can be done using
//! the [`lookup`] method on [`ActorSystemRef`], or if the type can't be typed
//! (which is the case when using asynchronous functions, see the methods
//! description for more info) [`lookup_val`] can be used instead. Both methods
//! will do the same thing; return a [`LocalActorRef`], which can be used to
//! communicate with the actor.
//!
//! ## Actor Registry Notes
//!
//! As the Actor Registry maps to `LocalActorRef`s each registry is **thread
//! local**. Example 3 (in the examples directory of the repo) shows this
//! possible gotcha in practice.
//!
//! Another thing to note is that the actor registration is **unique per actor
//! type** (within each thread). This means that two actors with the same type
//! cannot be registered at the same time, the second registration of the same
//! type will panic.
//!
//! [`NewActor`]: ../actor/trait.NewActor.html
//! [`LocalActorRef`]: ../actor_ref/struct.LocalActorRef.html
//! [register]: ./options/struct.ActorOptions.html#structfield.register
//! [`ActorOptions`]: ./options/struct.ActorOptions.html
//! [`spawn`]: ./struct.ActorSystemRef.html#method.spawn
//! [`Initiator`]: ../initiator/trait.Initiator.html
//! [`TcpListener`]: ../net/struct.TcpListener.html#method.bind
//! [`lookup`]: struct.ActorSystemRef.html#method.lookup
//! [`lookup_val`]: struct.ActorSystemRef.html#method.lookup_val

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::{fmt, io};

use crossbeam_channel::{self as channel, Receiver, Sender};
use log::{debug, trace};
use mio_st::event::{Evented, EventedId, Events, Ready};
use mio_st::poll::{Interests, PollOption, Poller};
use num_cpus;

use crate::actor::{Actor, ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::initiator::Initiator;
use crate::mailbox::MailBox;
use crate::scheduler::{ProcessId, Scheduler, SchedulerRef};
use crate::supervisor::Supervisor;
use crate::util::Shared;
use crate::waker::new_waker;

mod error;
mod registry;

use self::registry::ActorRegistry;

pub mod options;

pub use self::options::{ActorOptions, InitiatorOptions};
pub use self::error::RuntimeError;

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
/// start a single actor on each thread. This should print "Hello World" twice
/// (once on each thread started).
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
///
/// use heph::actor::ActorContext;
/// use heph::supervisor::NoopSupervisor;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
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
///     let new_actor = greeter_actor as fn(_, _) -> _;
///     let mut actor_ref = system_ref.spawn(NoopSupervisor, new_actor, "Hello", ActorOptions::default());
///
///     // Send a message to the actor.
///     actor_ref.send("World")?;
///     Ok(())
/// }
///
/// fn main() -> Result<(), RuntimeError> {
///     // Build a new `ActorSystem`.
///     ActorSystem::new()
///         // Start two worker threads.
///         .num_threads(2)
///         // On each worker thread run our setup function.
///         .with_setup(setup)
///         // And run the system.
///         .run()
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
        where S: FnOnce(ActorSystemRef) -> io::Result<()> + Send + Clone + 'static,
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

    /// Enable logging.
    ///
    /// See the [`log`] module for more information.
    ///
    /// # Panics
    ///
    /// This will panic if logging is already enabled by calling
    /// [`log::init`].
    ///
    /// [`log`]: ../log/index.html
    /// [`log::init`]: ../log/fn.init.html
    pub fn enable_logging(self) -> Self {
        crate::log::init();
        self
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
                    RuntimeError::panic(msg)
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
                    .map_err(RuntimeError::initiator)
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
        fn setup(self, system_ref: ActorSystemRef) -> io::Result<()>;
    }

    impl<F> SetupFn for F
        where F: FnOnce(ActorSystemRef) -> io::Result<()>
    {
        fn setup(self, system_ref: ActorSystemRef) -> io::Result<()> {
            (self)(system_ref)
        }
    }

    impl SetupFn for ! {
        fn setup(self, _: ActorSystemRef) -> io::Result<()> {
            self
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
        setup.setup(system_ref).map_err(RuntimeError::setup)?;
    }

    // All setup is done, so we're ready to run the event loop.
    actor_system.run_event_loop()
}

/// The system that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct RunningActorSystem {
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

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new() -> Result<RunningActorSystem, RuntimeError> {
        // Channel used in the `Waker` implementation.
        let (waker_send, waker_recv) = channel::unbounded();
        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();
        // System poller for event notifications.
        let poller = Poller::new().map_err(RuntimeError::poll)?;

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
        let pid = process_entry.pid();
        trace!("adding initiator to scheduler: pid={}", pid);

        // Initialise the initiator.
        trace!("initialising initiator: pid={}", pid);
        initiator.init(poller, pid).map_err(RuntimeError::initiator)?;

        // Add the process to the scheduler.
        process_entry.add_initiator(initiator);
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

        loop {
            let n_events = self.schedule_processes(&mut events)?;

            for _ in 0..RUN_POLL_RATIO {
                if !self.scheduler.run_process(&mut system_ref) {
                    if n_events == 0 {
                        // This is here to support actor system without
                        // initiators, e.g. example 1.
                        debug!("no events, no processes to run, stopping actor system");
                        return Ok(());
                    } else {
                        // No processes to run, try polling again.
                        break;
                    }
                }
            }
        }
    }

    /// Schedule processes.
    ///
    /// This polls the system poller and the waker notifications and schedules
    /// the processes notified.
    fn schedule_processes(&mut self, events: &mut Events) -> Result<usize, RuntimeError> {
        trace!("polling system poller for events");
        let timeout = self.determine_timeout();
        self.internal.borrow_mut().poller.poll(events, timeout)
            .map_err(RuntimeError::poll)?;

        // Schedule all processes with a notification.
        let n_events = events.len();
        for event in events {
            self.scheduler.schedule(event.id().into());
        }

        trace!("receiving waker events");
        while let Some(pid) = self.waker_notifications.try_recv() {
            self.scheduler.schedule(pid);
        }

        Ok(n_events)
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
    /// Spawn an actor on the actor system.
    ///
    /// Actors can never be unsupervised, so when adding an actor we need a good
    /// number of arguments. Starting with the `supervisor` of the actor, next
    /// is the way to create the actor, this is `new_actor`, and the `arg`ument
    /// to create it. Finally it also needs `options` for actor and the place
    /// inside the actor system.
    pub fn spawn<S, NA>(&mut self, supervisor: S, new_actor: NA, arg: NA::Argument, options: ActorOptions) -> LocalActorRef<NA::Message>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              NA::Actor: 'static,
    {
        self.add_actor_setup(supervisor, new_actor, |_, _| Ok(arg), options)
            .unwrap() // Safe because the argument function doesn't return an error.
    }

    /// Add an actor that needs to be initialised.
    ///
    /// This is used by the `Initiator`s to register with the system poller with
    /// using same pid.
    ///
    /// Just like `add_actor` this required a `supervisor`, `new_actor` and
    /// `options`. The only difference being rather then passing an argument
    /// directly this function requires a function to create the argument, which
    /// allows the caller to do any required setup work.
    pub(crate) fn add_actor_setup<S, NA, ArgFn>(&mut self, supervisor: S, new_actor: NA, arg_fn: ArgFn, options: ActorOptions) -> io::Result<LocalActorRef<NA::Message>>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              ArgFn: FnOnce(ProcessId, &mut Poller) -> io::Result<NA::Argument>,
              NA::Actor: 'static,
    {
        let system_ref = self.clone();
        self.internal.borrow_mut().add_actor(supervisor, new_actor, arg_fn, options, system_ref)
    }

    /// Lookup an actor in the [Actor Registry].
    ///
    /// This looks up an actor in Actor Registry and returns an actor reference
    /// to that actor, if found. For more information, as well as possible
    /// gotchas of the design, see [Actor Registry].
    ///
    /// Note: when using asynchronous functions as actors you'll likely need
    /// [`lookup_val`].
    ///
    /// [Actor Registry]: ./index.html#actor-registry
    /// [`lookup_val`]: #method.lookup_val
    pub fn lookup<NA>(&mut self) -> Option<LocalActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        self.internal.borrow_mut().registry.lookup::<NA>()
    }

    /// Lookup an actor in the [Actor Registry], of which the type is hard to
    /// get.
    ///
    /// Actors can be implemented as an asynchronous function, of which it hard
    /// to get type, e.g. `system_ref.lookup::<my_actor>()` will not work. This
    /// is because `my_actor` is not a type (it is a [function]) and doesn't
    /// implement [`NewActor`] directly, only function pointers do.
    ///
    /// This function is a work around for that problem, although it isn't
    /// pretty. See the example below for usage.
    ///
    /// Note: the argument provided is completely ignored and only used to
    /// determine the type of the argument.
    ///
    /// [Actor Registry]: ./index.html#actor-registry
    /// [function]: https://doc.rust-lang.org/std/keyword.fn.html
    /// [`NewActor`]: ../actor/trait.NewActor.html
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use std::io;
    ///
    /// use heph::actor::ActorContext;
    /// use heph::supervisor::NoopSupervisor;
    /// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
    ///
    /// /// Our actor implemented as an asynchronous function.
    /// async fn actor(mut ctx: ActorContext<()>) -> Result<(), !> {
    ///     // ...
    ///     # Ok(())
    /// }
    ///
    /// /// Setup function used in starting the `ActorSystem`.
    /// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    ///     // Add the actor to the system, enabling registering of the actor.
    ///     let actor_ref1 = system_ref.spawn(NoopSupervisor, actor as fn(_) -> _,
    ///         (), ActorOptions { register: true, .. ActorOptions::default() });
    ///
    ///     // Unfortunately this won't compile. :(
    ///     //let actor_ref2 = system_ref.lookup::<actor>();
    ///
    ///     // This will work.
    ///     let actor_ref2 = system_ref.lookup_val(&(actor as fn(_) -> _))
    ///         .unwrap();
    ///
    ///     // The actor reference should be the same.
    ///     assert_eq!(actor_ref1, actor_ref2);
    ///     Ok(())
    /// }
    ///
    /// fn main() -> Result<(), RuntimeError> {
    ///     ActorSystem::new()
    /// #       .num_threads(1)
    ///         .with_setup(setup)
    ///         .run()
    /// }
    /// ```
    pub fn lookup_val<NA>(&mut self, _: &NA) -> Option<LocalActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        self.lookup::<NA>()
    }

    /// Deregister an actor.
    pub(crate) fn deregister<NA>(&mut self)
        where NA: NewActor + 'static,
    {
        let _ = self.internal.borrow_mut().registry.deregister::<NA>();
    }

    /// Register an `Evented` handle, see `Poll.register`.
    pub(crate) fn poller_register<E>(&mut self, handle: &mut E, id: EventedId, interests: Interests, opt: PollOption) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        self.internal.borrow_mut().poller.register(handle, id, interests, opt)
    }

    /// Get a clone of the sending end of the notification channel.
    pub(crate) fn get_notification_sender(&mut self) -> Sender<ProcessId> {
        self.internal.borrow().waker_notifications.clone()
    }

    /// Add a deadline to the system poller.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.internal.borrow_mut().poller.add_deadline(pid.into(), deadline);
    }

    pub(crate) fn notify(&mut self, pid: ProcessId) {
        self.internal.borrow_mut().poller.notify(pid.into(), Ready::READABLE);
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
    /// Actor registrations.
    registry: ActorRegistry,
}

impl ActorSystemInternal {
    /// Create new actor system internals.
    pub fn new(scheduler_ref: SchedulerRef, poller: Poller, waker_notifications: Sender<ProcessId>) -> ActorSystemInternal {
        ActorSystemInternal {
            scheduler_ref,
            poller,
            waker_notifications,
            registry: ActorRegistry::new(),
        }
    }

    pub fn add_actor<S, NA, ArgFn>(&mut self, supervisor: S, mut new_actor: NA, arg_fn: ArgFn, options: ActorOptions, system_ref: ActorSystemRef) -> io::Result<LocalActorRef<NA::Message>>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              ArgFn: FnOnce(ProcessId, &mut Poller) -> io::Result<NA::Argument>,
    {
        // Setup adding a new process to the scheduler.
        let process_entry = self.scheduler_ref.add_process();
        let pid = process_entry.pid();
        debug!("adding actor to actor system: pid={}", pid);

        // Create our waker, mailbox and actor reference.
        let waker = new_waker(pid, self.waker_notifications.clone());
        let mailbox = Shared::new(MailBox::new(pid, system_ref.clone()));
        let actor_ref = LocalActorRef::new(mailbox.downgrade());

        // Create the actor context, the argument for the actor and create an
        // actor with both.
        let ctx = ActorContext::new(pid, system_ref, mailbox.clone());
        let arg = arg_fn(pid, &mut self.poller)?;
        let actor = new_actor.new(ctx, arg);

        // Add the actor to the scheduler.
        process_entry.add_actor(options.priority, supervisor, new_actor, actor, mailbox, waker);

        if options.register {
            if self.registry.register::<NA>(actor_ref.clone()).is_some() {
                panic!("can't overwrite a previously registered actor in the Actor Registry");
            }
        }
        if options.schedule {
            self.waker_notifications.send(pid);
        }

        Ok(actor_ref)
    }
}
