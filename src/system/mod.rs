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
//! [`spawn`]. Note that an actor registration must be unique **per actor
//! type**, that is type in the Rust's type system. It is not possible to
//! register two actors with the same type.
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
//! [`lookup`]: struct.ActorSystemRef.html#method.lookup
//! [`lookup_val`]: struct.ActorSystemRef.html#method.lookup_val

use std::cell::RefCell;
use std::rc::Rc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::{fmt, io};

use crossbeam_channel::{self as channel, Receiver, Sender};
use log::{debug, trace};
use mio_st::event::{Evented, EventedId, Events, Ready};
use mio_st::poll::{Interests, PollOption, Poller};
use num_cpus;

use crate::actor::{Actor, ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::scheduler::{ProcessId, Scheduler, SchedulerRef};
use crate::supervisor::Supervisor;
use crate::util::Shared;
use crate::waker::new_waker;

mod error;
mod registry;

use self::registry::ActorRegistry;

pub mod options;

pub use self::error::RuntimeError;
pub use self::options::ActorOptions;

/// The system that runs all actors.
///
/// This type implements a builder pattern to build and run an actor system. It
/// has a single generic parameter `S`, the type of the setup function. The
/// setup function is optional, which is represented by `!` (the never type).
///
/// ## Usage
///
/// Building an actor system starts with calling [`new`], this will create a new
/// system without a setup function, using a single worker thread.
///
/// An optional setup function can be added. This setup function will be called
/// on each thread the actor system starts, and thus has to be `Clone` and
/// `Send`. This can be done by calling [`with_setup`]. This will change the `S`
/// parameter from `!` to an actual type. Only a single setup function can be
/// used per actor system.
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
/// [`new`]: #method.new
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
/// use heph::supervisor::NoSupervisor;
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
///     let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, "Hello", ActorOptions::default())
///         // This is safe because the `NewActor` implementation for
///         // asynchronous functions never returns an error.
///         .unwrap();
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
}

impl ActorSystem<!> {
    /// Add a setup function.
    ///
    /// This function will be run on each thread the actor system creates. Only
    /// a single setup function can be added to the actor system.
    pub fn with_setup<S>(self, setup: S) -> ActorSystem<S>
        where S: FnOnce(ActorSystemRef) -> io::Result<()> + Send + Clone + 'static,
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
    /// Most applications would want to use [`use_all_cores`] which sets the
    /// number of threads equal to the number of cpu cores.
    ///
    /// [`use_all_cores`]: #method.use_all_cores
    pub fn num_threads(mut self, n: usize) -> Self {
        self.threads = n;
        self
    }

    /// Set the number of worker threads equal to the number of cpu cores.
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
}

impl<S> ActorSystem<S>
    where S: SetupFn  + Send + Clone + 'static,
{
    /// Run the system.
    ///
    /// This will spawn a number of threads (see [`num_threads`]) to run the
    /// system.
    ///
    /// [`num_threads`]: #method.num_threads
    pub fn run(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system: worker_threads={}", self.threads);

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
            // Spawn the thread.
            let setup = self.setup.clone();
            Ok(thread::spawn(move || run_system(setup)))
        })
        .collect()
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

/// Run the actor system, with the optional `setup` function.
///
/// This is the entry point for the worker threads.
fn run_system<S>(setup: Option<S>) -> Result<(), RuntimeError>
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
        let internal = ActorSystemInternal {
            scheduler_ref: RefCell::new(scheduler_ref),
            poller: RefCell::new(poller),
            waker_notifications: waker_send,
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
    pub fn run_event_loop(mut self) -> Result<(), RuntimeError> {
        debug!("running actor system's event loop");

        // The schedule field in `ActorOptions` will send a waker notification,
        // so we need to check for those any schedule any actors that start
        // scheduled.
        while let Some(pid) = self.waker_notifications.try_recv() {
            self.scheduler.schedule(pid);
        }

        // Empty set of events, to be filled by the system poller.
        let mut events = Events::new();
        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        loop {
            // First run any processes that are ready, only after that we poll.
            for _ in 0..RUN_POLL_RATIO {
                if !self.scheduler.run_process(&mut system_ref) {
                    if self.scheduler.is_empty() {
                        // No processes left to run, so we're done.
                        debug!("no processes to run, stopping actor system");
                        return Ok(())
                    } else {
                        // No processes ready to run, try polling again.
                        break;
                    }
                }
            }

            self.schedule_processes(&mut events)?;
        }
    }

    /// Schedule processes.
    ///
    /// This polls the system poller and the waker notifications and schedules
    /// the processes notified.
    fn schedule_processes(&mut self, events: &mut Events) -> Result<(), RuntimeError> {
        trace!("polling system poller for events");
        self.internal.poller.borrow_mut().poll(events, None)
            .map_err(RuntimeError::poll)?;

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
    internal: Rc<ActorSystemInternal>,
}

impl ActorSystemRef {
    /// Spawn an actor on the actor system.
    ///
    /// Actors can never be unsupervised, so when adding an actor we need a good
    /// number of arguments. Starting with the `supervisor` of the actor, next
    /// is the way to create the actor, this is `new_actor`, and the `arg`ument
    /// to create it. Finally it also needs `options` for actor and the place
    /// inside the actor system.
    pub fn spawn<S, NA>(&mut self, supervisor: S, new_actor: NA, arg: NA::Argument, options: ActorOptions) -> Result<LocalActorRef<NA::Message>, NA::Error>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              NA::Actor: 'static,
    {
        self.add_actor_setup(supervisor, new_actor, |_, _| Ok(arg), options)
            .map_err(|err| match err {
                AddActorError::NewActor(err) => err,
                AddActorError::<_, !>::ArgFn(_) => unreachable!(),
            })
    }

    /// Add an actor that needs to be initialised.
    ///
    /// Just like `add_actor` this required a `supervisor`, `new_actor` and
    /// `options`. The only difference being rather then passing an argument
    /// directly this function requires a function to create the argument, which
    /// allows the caller to do any required setup work.
    pub(crate) fn add_actor_setup<S, NA, ArgFn, ArgFnE>(&mut self, supervisor: S,
        mut new_actor: NA, arg_fn: ArgFn, options: ActorOptions
    ) -> Result<LocalActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
              ArgFn: FnOnce(ProcessId, &mut ActorSystemRef) -> Result<NA::Argument, ArgFnE>,
              NA::Actor: 'static,
    {
        let ActorSystemInternal {
            ref scheduler_ref,
            ref waker_notifications,
            ref registry,
            ..
        } = &*self.internal;

        // Setup adding a new process to the scheduler.
        let mut scheduler_ref = scheduler_ref.borrow_mut();
        let process_entry = scheduler_ref.add_process();
        let pid = process_entry.pid();
        debug!("adding actor to actor system: pid={}", pid);

        // Create our waker, mailbox and actor reference.
        let waker = new_waker(pid, waker_notifications.clone());
        let mailbox = Shared::new(MailBox::new(pid, self.clone()));
        let actor_ref = LocalActorRef::new(mailbox.downgrade());

        // Create the argument for the actor, the actor context and create the
        // actor with both.
        let mut system_ref = self.clone();
        let arg = arg_fn(pid, &mut system_ref)
            .map_err(|err| AddActorError::ArgFn(err))?;
        let ctx = ActorContext::new(pid, system_ref, mailbox.clone());
        let actor = new_actor.new(ctx, arg)
            .map_err(|err| AddActorError::NewActor(err))?;

        // Add the actor to the scheduler.
        process_entry.add_actor(options.priority, supervisor, new_actor, actor,
            mailbox, waker);

        if options.register {
            if registry.borrow_mut().register::<NA>(actor_ref.clone()).is_some() {
                panic!("can't overwrite a previously registered actor in the Actor Registry");
            }
        }
        if options.schedule {
            waker_notifications.send(pid);
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
    /// [`lookup_val`].
    ///
    /// [Actor Registry]: ./index.html#actor-registry
    /// [`lookup_val`]: #method.lookup_val
    pub fn lookup<NA>(&mut self) -> Option<LocalActorRef<NA::Message>>
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
    /// use heph::supervisor::NoSupervisor;
    /// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
    ///
    /// /// Our actor implemented as an asynchronous function.
    /// async fn actor(mut ctx: ActorContext<()>) -> Result<(), !> {
    ///     // ...
    /// #   Ok(())
    /// }
    ///
    /// /// Setup function used in starting the `ActorSystem`.
    /// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    ///     // Add the actor to the system, enabling registering of the actor.
    ///     let mut actor_ref1 = system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (), ActorOptions {
    ///         register: true,
    ///         .. ActorOptions::default()
    ///     }).unwrap();
    /// #   // Actually run the actor, so the example can run.
    /// #   actor_ref1.send(()).unwrap();
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
    ///         .with_setup(setup)
    ///         .run()
    /// }
    /// ```
    pub fn lookup_val<NA>(&mut self, _: &NA) -> Option<LocalActorRef<NA::Message>>
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
    pub(crate) fn get_notification_sender(&mut self) -> Sender<ProcessId> {
        self.internal.waker_notifications.clone()
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
pub(crate) enum AddActorError<NewActorE, ArgFnE> {
    /// Calling new actor resulted in an error.
    NewActor(NewActorE),
    /// Calling the argument function resulted in an error.
    ArgFn(ArgFnE),
}

/// Internals of the `RunningActorSystem`, to which `ActorSystemRef`s have a
/// reference.
#[derive(Debug)]
pub(crate) struct ActorSystemInternal {
    /// A reference to the scheduler to add new processes to.
    scheduler_ref: RefCell<SchedulerRef>,
    /// System poller, used for event notifications to support non-blocking I/O.
    poller: RefCell<Poller>,
    /// Sending side of the channel for `Waker` notifications.
    waker_notifications: Sender<ProcessId>,
    /// Actor registrations.
    registry: RefCell<ActorRegistry>,
}
