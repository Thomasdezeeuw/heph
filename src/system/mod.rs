//! Module with the actor system and related types.
//!
//! The module has two main types and a submodule:
//!
//! - [`ActorSystem`] is the actor system, used to run all actors.
//! - [`ActorSystemRef`] is a reference to a running actor system, used for
//!   example to spawn new actors or get access to the [Actor Registry].
//!
//! See [`ActorSystem`] for documentation on how to run an actor system. For
//! more examples see the examples directory in the source code.
//!
//! [Actor Registry]: #actor-registry
//!
//! # Actor Registry
//!
//! The Actor Registry is a fairly simple concept, it maps a [`NewActor`] type
//! to an [`ActorRef`].
//!
//! First an actor must be registered with the Actor Registry. This is done by
//! setting the [register] option to `true` in the [`ActorOptions`] when
//! [spawning] an actor. Note that an actor registration must be unique **per
//! actor type**, that is type in the Rust's type system. It is not possible to
//! register two actors with the same type.
//!
//! After the actor is registered it can be looked up. This can be done using
//! the [`lookup`] method, or if the type can't be typed (which is the case when
//! using asynchronous functions, see the methods description for more info)
//! [`lookup_actor`] can be used instead. Both methods will do the same thing
//! return an [`ActorRef`], which can be used to communicate with the actor.
//!
//! ## Actor Registry Notes
//!
//! As the Actor Registry maps to `ActorRef`s each registry is **thread local**.
//! Example 3 (in the examples directory of the repo) shows this possible gotcha
//! in practice.
//!
//! Another thing to note is that the actor registration is **unique per actor
//! type** (within each thread). This means that two actors with the same type
//! cannot be registered at the same time, the second registration of the same
//! type will panic.
//!
//! [register]: crate::system::options::ActorOptions::register
//! [spawning]: ActorSystemRef::try_spawn
//! [`lookup`]: ActorSystemRef::lookup
//! [`lookup_actor`]: ActorSystemRef::lookup_actor

use std::cell::RefCell;
use std::rc::Rc;
use std::task::Waker;
use std::time::Instant;
use std::{fmt, io, thread};

use crossbeam_channel::{self as channel, Receiver};
use log::{debug, trace};
use mio_st::event::{Evented, EventedId, Events, Ready};
use mio_st::poll::{Awakener, Interests, PollOption, Poller};
use num_cpus;

use crate::actor::{Actor, Context, NewActor};
use crate::actor_ref::{ActorRef, Local};
use crate::mailbox::MailBox;
use crate::scheduler::{ProcessId, Scheduler, SchedulerRef};
use crate::supervisor::Supervisor;
use crate::util::Shared;

mod error;
mod registry;
mod waker;

use self::registry::ActorRegistry;

pub mod options;

pub use self::error::RuntimeError;
pub use self::options::ActorOptions;

use waker::{MAX_THREADS, WakerId, new_waker, init_waker};

/// The system that runs all actors.
///
/// This type implements a builder pattern to build and run an actor system. It
/// has a single generic parameter `S`, the type of the setup function. The
/// setup function is optional, which is represented by `!` (the never type).
///
/// The actor system will start workers threads that will run all actors, these
/// threads will run until all actors have returned.
///
/// ## Usage
///
/// Building an actor system starts with calling [`new`], this will create a new
/// system without a setup function, using a single worker thread.
///
/// An optional setup function can be added. This setup function will be called
/// on each thread the actor system starts, and thus has to be [`Clone`] and
/// [`Send`]. This can be done by calling [`with_setup`]. This will change the
/// `S` parameter from `!` to an actual type. Only a single setup function can
/// be used per actor system.
///
/// Now that the generic parameter is optionally defined we also have some more
/// configuration options. One such option is the number of threads the system
/// uses, this can configured with the [`num_threads`] method, or
/// [`use_all_cores`].
///
/// Finally after setting all the configuration options the system can be
/// [`run`]. This will spawn a number of worker threads to actually run the
/// system.
///
/// [`new`]: ActorSystem::new
/// [`with_setup`]: ActorSystem::with_setup
/// [`num_threads`]: ActorSystem::num_threads
/// [`use_all_cores`]: ActorSystem::use_all_cores
/// [`run`]: ActorSystem::run
///
/// ## Examples
///
/// This simple example shows how to run an `ActorSystem` and add how start a
/// single actor on each thread. This should print "Hello World" twice (once on
/// each worker thread started).
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use heph::actor::Context;
/// use heph::supervisor::NoSupervisor;
/// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
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
///
/// // This setup function will on run on each created thread. In the case of
/// // this example we create 2 threads (see `main`).
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
///     // Asynchronous function don't yet implement the required `NewActor`
///     // trait, but function pointers do.
///     let new_actor = greeter_actor as fn(_, _) -> _;
///     // Each actors needs to be supervised, but since our actor doesn't return
///     // an error we can get away with our `NoSupervisor` which doesn't do
///     // anything.
///     let supervisor = NoSupervisor;
///     // All actors start with one or more arguments, in our case it's message
///     // used to greet people with. See `greeter_actor` below.
///     let arg = "Hello";
///     // Spawn the actor to run on our actor system.
///     let mut actor_ref = system_ref.spawn(supervisor, new_actor, arg, ActorOptions::default());
///
///     // Send a message to the actor we just spawned with it's `ActorRef`.
///     actor_ref <<= "World";
///
///     Ok(())
/// }
///
/// /// Our actor that greets people.
/// async fn greeter_actor(mut ctx: Context<&'static str>, message: &'static str) -> Result<(), !> {
///     // `message` is the argument passed to `spawn` in the `setup` function
///     // above, in this example it was "Hello".
///
///     // Using the context we use receive messages for this actor, so here
///     // we'll receive the "World" message we send in the setup function.
///     let name = await!(ctx.receive_next());
///     // This should print "Hello world"!
///     println!("{} {}", message, name);
///     Ok(())
/// }
/// ```
pub struct ActorSystem<S = !> {
    /// Number of threads to create.
    threads: usize,
    /// Optional setup function.
    setup: Option<S>,
}

impl ActorSystem {
    /// Create a new actor system with the default configuration.
    pub fn new() -> ActorSystem<!> {
        ActorSystem {
            threads: 1,
            setup: None,
        }
    }

    /// Add a setup function.
    ///
    /// This function will be run on each worker thread the actor system
    /// creates. Only a single setup function can be added to the actor system.
    pub fn with_setup<F, E>(self, setup: F) -> ActorSystem<F>
        where F: FnOnce(ActorSystemRef) -> Result<(), E> + Send + Clone + 'static,
              E: Send,
    {
        ActorSystem {
            threads: self.threads,
            setup: Some(setup),
        }
    }
}

impl<S> ActorSystem<S> {
    /// Set the number of worker threads to use, defaults to `1`.
    ///
    /// Most applications would want to use [`ActorSystem::use_all_cores`] which
    /// sets the number of threads equal to the number of CPU cores.
    pub fn num_threads(mut self, n: usize) -> Self {
        if n > MAX_THREADS {
            panic!("Can't create {} worker threads, {} is the maximum", n, MAX_THREADS);
        }
        self.threads = n;
        self
    }

    /// Set the number of worker threads equal to the number of CPU cores.
    ///
    /// See [`ActorSystem::num_threads`].
    pub fn use_all_cores(self) -> Self {
        self.num_threads(num_cpus::get())
    }
}

impl<S> ActorSystem<S>
    where S: SetupFn,
{
    /// Run the system.
    ///
    /// This will spawn a number of worker threads (see
    /// [`ActorSystem::num_threads`]) to run the system.
    pub fn run(self) -> Result<(), RuntimeError<S::Error>> {
        debug!("running actor system: worker_threads={}", self.threads);

        let mut handles = Vec::with_capacity(self.threads);
        for id in 0..self.threads {
            let setup = self.setup.clone();
            let handle = thread::Builder::new()
                .name(format!("heph_worker{}", id))
                .spawn(move || run_system(setup))
                .map_err(RuntimeError::start_thread)?;
            handles.push(handle)
        }

        // TODO: handle errors better. Currently as long as the first thread
        // keeps running and all other threads crash we'll keep running. Not an
        // ideal situation.
        for handle in handles {
            handle.join()
                .map_err(|err| match err.downcast_ref::<&'static str>() {
                    Some(s) => (*s).to_owned(),
                    None => match err.downcast_ref::<String>() {
                        Some(s) => s.clone(),
                        None => "unkown panic message".to_owned(),
                    },
                })
                .map_err(RuntimeError::panic)
                .and_then(|res| res)?;
        }

        Ok(())
    }
}

impl<S> fmt::Debug for ActorSystem<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorSystem")
            .field("threads", &self.threads)
            .field("setup", if self.setup.is_some() { &"Some" } else { &"None" })
            .finish()
    }
}

/// A hack to allow us to call `ActorSystem<!>.run`.
mod hack {
    use super::ActorSystemRef;

    pub trait SetupFn: Send + Clone + 'static {
        type Error: Send;

        fn setup(self, system_ref: ActorSystemRef) -> Result<(), Self::Error>;
    }

    impl<F, E> SetupFn for F
        where F: FnOnce(ActorSystemRef) -> Result<(), E> + Send + Clone + 'static,
              E: Send,
    {
        type Error = E;

        fn setup(self, system_ref: ActorSystemRef) -> Result<(), Self::Error> {
            (self)(system_ref)
        }
    }

    impl SetupFn for ! {
        type Error = !;

        fn setup(self, _: ActorSystemRef) -> Result<(), Self::Error> {
            self
        }
    }
}

use self::hack::SetupFn;

/// Run the actor system, with an optional `setup` function.
///
/// This is the entry point for the worker threads.
fn run_system<S>(setup: Option<S>) -> Result<(), RuntimeError<S::Error>>
    where S: SetupFn,
{
    let actor_system = RunningActorSystem::new()?;

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
    internal: Rc<ActorSystemInternal>,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// Receiving side of the channel for `Waker` notifications.
    waker_notifications: Receiver<ProcessId>,
}

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Id used for the awakener.
const AWAKENER_ID: EventedId = EventedId(usize::max_value());

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new<E>() -> Result<RunningActorSystem, RuntimeError<E>> {
        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = channel::unbounded();
        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();
        // System poller for event notifications.
        let mut poller = Poller::new().map_err(RuntimeError::poll)?;
        let awakener = Awakener::new(&mut poller, AWAKENER_ID)
            .map_err(RuntimeError::poll)?;

        let waker_id = init_waker(awakener, waker_sender);

        // Internals of the running actor system.
        let internal = ActorSystemInternal {
            waker_id,
            scheduler_ref: RefCell::new(scheduler_ref),
            poller: RefCell::new(poller),
            registry: RefCell::new(ActorRegistry::new()),
        };

        // The actor system we're going to run.
        Ok(RunningActorSystem {
            internal: Rc::new(internal),
            scheduler,
            waker_notifications: waker_recv,
        })
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            internal: self.internal.clone(),
        }
    }

    /// Run the actor system's event loop.
    pub fn run_event_loop<E>(mut self) -> Result<(), RuntimeError<E>> {
        debug!("running actor system's event loop");

        // Empty set of events, to be filled by the system poller.
        let mut events = Events::new();
        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        loop {
            self.schedule_processes(&mut events)?;

            for _ in 0 .. RUN_POLL_RATIO {
                if !self.scheduler.run_process(&mut system_ref) {
                    if self.scheduler.is_empty() {
                        // No processes left to run, so we're done.
                        debug!("no processes to run, stopping actor system");
                        return Ok(());
                    } else {
                        // No processes ready to run, try polling again.
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
    fn schedule_processes<E>(&mut self, events: &mut Events) -> Result<(), RuntimeError<E>> {
        trace!("polling system poller for events");
        self.internal.poller.borrow_mut().poll(events, None)
            .map_err(RuntimeError::poll)?;

        // Schedule all processes with a notification.
        for event in events {
            if event.id() == AWAKENER_ID {
                trace!("receiving waker events");
                while let Some(pid) = self.waker_notifications.try_recv().ok() {
                    self.scheduler.schedule(pid);
                }
                continue;
            }

            self.scheduler.schedule(event.id().into());
        }

        Ok(())
    }
}

/// A reference to an [`ActorSystem`].
///
/// A reference to the running actor system can be accessed via the actor
/// [`Context::system_ref`].
///
/// This reference refers to the thread-local actor system, and thus can't be
/// shared across thread bounds. To share this reference (within the same
/// thread) it can be cloned.
#[derive(Clone, Debug)]
pub struct ActorSystemRef {
    /// A shared reference to the actor system's internals.
    internal: Rc<ActorSystemInternal>,
}

impl ActorSystemRef {
    /// Attempts to spawn an actor on the actor system.
    ///
    /// Actors can never be unsupervised, so when adding an actor we need a good
    /// number of arguments. Starting with the `supervisor` of the actor, next
    /// is the way to create the actor, this is `new_actor`, and the `arg`ument
    /// to create it. Finally it also needs `options` for actor and the place
    /// inside the actor system.
    pub fn try_spawn<S, NA>(&mut self, supervisor: S, new_actor: NA, arg: NA::Argument, options: ActorOptions) -> Result<ActorRef<NA::Message>, NA::Error>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              NA::Actor: 'static,
    {
        self.try_spawn_setup(supervisor, new_actor, |_, _| Ok(arg), options)
            .map_err(|err| match err {
                AddActorError::NewActor(err) => err,
                AddActorError::<_, !>::ArgFn(_) => unreachable!(),
            })
    }

    /// Spawn an actor on the actor system.
    ///
    /// This is a convenience method for `NewActor` implementations that never
    /// return an error, such as asynchronous functions.
    ///
    /// See [`ActorSystemRef::try_spawn`] for more information.
    pub fn spawn<S, NA>(&mut self, supervisor: S, new_actor: NA, arg: NA::Argument, options: ActorOptions) -> ActorRef<NA::Message>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor<Error = !> + 'static,
              NA::Actor: 'static,
    {
        self.try_spawn_setup(supervisor, new_actor, |_, _| Ok(arg), options)
            .unwrap_or_else(|_: AddActorError<!, !>| unreachable!())
    }

    /// Spawn an actor that needs to be initialised.
    ///
    /// Just like `try_spawn` this requires a `supervisor`, `new_actor` and
    /// `options`. The only difference being rather then passing an argument
    /// directly this function requires a function to create the argument, which
    /// allows the caller to do any required setup work.
    pub(crate) fn try_spawn_setup<S, NA, ArgFn, ArgFnE>(&mut self, supervisor: S,
        mut new_actor: NA, arg_fn: ArgFn, options: ActorOptions
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              ArgFn: FnOnce(ProcessId, &mut ActorSystemRef) -> Result<NA::Argument, ArgFnE>,
              NA::Actor: 'static,
    {
        let ActorSystemInternal {
            waker_id,
            ref scheduler_ref,
            ref registry,
            ..
        } = &*self.internal;

        // Setup adding a new process to the scheduler.
        let mut scheduler_ref = scheduler_ref.borrow_mut();
        let process_entry = scheduler_ref.add_process();
        let pid = process_entry.pid();
        debug!("adding actor to actor system: pid={}", pid);

        // Create our actor argument, running any setup required by the caller.
        let mut system_ref = self.clone();
        let arg = arg_fn(pid, &mut system_ref).map_err(AddActorError::ArgFn)?;

        // Create our actor context and our actor with it.
        let mailbox = Shared::new(MailBox::new(pid, self.clone()));
        let actor_ref = ActorRef::<NA::Message, Local>::new(mailbox.downgrade());
        let ctx = Context::new(pid, system_ref, mailbox.clone());
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        let waker = new_waker(*waker_id, pid);
        if options.schedule {
            waker.wake()
        }

        // Add the actor to the scheduler.
        process_entry.add_actor(options.priority, supervisor, new_actor, actor,
            mailbox, waker);

        if options.register && registry.borrow_mut().register::<NA>(actor_ref.clone()).is_some() {
            panic!("can't overwrite a previously registered actor in the Actor Registry");
        }

        Ok(actor_ref)
    }

    /// Lookup an actor in the [Actor Registry].
    ///
    /// This looks up an actor in Actor Registry and returns an actor reference
    /// to that actor, if found. For more information, as well as possible
    /// gotchas of the design, see [Actor Registry].
    ///
    /// Note: when using asynchronous functions as actors you'll likely need
    /// [`ActorSystemRef::lookup_actor`].
    ///
    /// [Actor Registry]: ./index.html#actor-registry
    pub fn lookup<NA>(&mut self) -> Option<ActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        self.internal.registry.borrow_mut().lookup::<NA>()
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
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use heph::actor::Context;
    /// use heph::supervisor::NoSupervisor;
    /// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
    ///
    /// fn main() -> Result<(), RuntimeError> {
    ///     ActorSystem::new()
    ///         .with_setup(setup)
    ///         .run()
    /// }
    ///
    /// /// Setup function used in starting the `ActorSystem`.
    /// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
    ///     // Add the actor to the system, enabling registering of the actor.
    ///     let actor_ref1 = system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (), ActorOptions {
    ///         register: true,
    /// #       schedule: true, // Run the actor, so the example runs.
    ///         .. ActorOptions::default()
    ///     });
    ///
    ///     // Unfortunately this won't compile. :(
    ///     //let actor_ref2 = system_ref.lookup::<actor>();
    ///
    ///     // This will work.
    ///     let actor_ref2 = system_ref.lookup_actor(&(actor as fn(_) -> _))
    ///         .unwrap();
    ///
    ///     // The actor reference should be the same.
    ///     assert_eq!(actor_ref1, actor_ref2);
    ///     Ok(())
    /// }
    ///
    /// /// Our actor implemented as an asynchronous function.
    /// async fn actor(ctx: Context<()>) -> Result<(), !> {
    ///     // ...
    /// #   drop(ctx); // Silence dead code warnings.
    /// #   Ok(())
    /// }
    /// ```
    pub fn lookup_actor<NA>(&mut self, _: &NA) -> Option<ActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        self.lookup::<NA>()
    }

    /// Deregister an actor from the Actor Registry.
    pub(crate) fn deregister<NA>(&mut self)
        where NA: NewActor + 'static,
    {
        let _ = self.internal.registry.borrow_mut().deregister::<NA>();
    }

    /// Register an `Evented` handle, see `Poll.register`.
    pub(crate) fn poller_register<E>(&mut self, handle: &mut E, id: EventedId, interests: Interests, opt: PollOption) -> io::Result<()>
    where
        E: Evented + ?Sized,
    {
        self.internal.poller.borrow_mut().register(handle, id, interests, opt)
    }

    /// Get a clone of the sending end of the notification channel.
    pub(crate) fn new_waker(&self, pid: ProcessId) -> Waker {
        new_waker(self.internal.waker_id, pid)
    }

    /// Add a deadline to the system poller.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.internal.poller.borrow_mut().add_deadline(pid.into(), deadline);
    }

    pub(crate) fn notify(&mut self, pid: ProcessId) {
        self.internal.poller.borrow_mut().notify(pid.into(), Ready::READABLE);
    }
}

/// Internal error returned by adding a new actor to the actor system.
#[derive(Debug)]
pub(crate) enum AddActorError<NewActorE, ArgFnE> {
    /// Calling `NewActor::new` actor resulted in an error.
    NewActor(NewActorE),
    /// Calling the argument function resulted in an error.
    ArgFn(ArgFnE),
}

/// Internals of the `RunningActorSystem`, to which `ActorSystemRef`s have a
/// reference.
#[derive(Debug)]
pub(crate) struct ActorSystemInternal {
    /// Waker id used to create a `Waker`.
    waker_id: WakerId,
    /// A reference to the scheduler to add new processes to.
    scheduler_ref: RefCell<SchedulerRef>,
    /// System poller, used for event notifications to support non-blocking I/O.
    poller: RefCell<Poller>,
    /// Actor registrations.
    registry: RefCell<ActorRegistry>,
}
