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

use std::any::Any;
use std::cell::RefCell;
use std::ops::DerefMut;
use std::ptr::NonNull;
use std::rc::Rc;
use std::task::Waker;
use std::time::{Duration, Instant};
use std::{fmt, io, thread};

use crossbeam_channel::{self as channel, Receiver};
use gaea::os::{Awakener, Evented, Interests, OsQueue, RegisterOption};
use gaea::{event, poll, Event, Queue, Ready, Timers};
use log::{debug, trace};
use num_cpus;

use crate::actor::sync::{SyncActor, SyncContext, SyncContextData};
use crate::actor_ref::{ActorRef, Local, Sync};
use crate::mailbox::MailBox;
use crate::scheduler::{ProcessId, Scheduler, SchedulerRef};
use crate::supervisor::{Supervisor, SupervisorStrategy};
use crate::util::Shared;
use crate::{actor, Actor, NewActor};

mod error;
mod waker;

pub mod options;

pub use self::error::RuntimeError;
pub use self::options::ActorOptions;

use waker::{init_waker, new_waker, WakerId, MAX_THREADS};

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
/// #![feature(async_await, never_type)]
///
/// use heph::supervisor::NoSupervisor;
/// use heph::system::RuntimeError;
/// use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef};
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
/// async fn greeter_actor(mut ctx: actor::Context<&'static str>, message: &'static str) -> Result<(), !> {
///     // `message` is the argument passed to `spawn` in the `setup` function
///     // above, in this example it was "Hello".
///
///     // Using the context we use receive messages for this actor, so here
///     // we'll receive the "World" message we send in the setup function.
///     let name = ctx.receive_next().await;
///     // This should print "Hello world"!
///     println!("{} {}", message, name);
///     Ok(())
/// }
/// ```
pub struct ActorSystem<S = !> {
    /// Number of threads to create.
    threads: usize,
    /// Synchronous actor thread handles.
    sync_actors: Vec<thread::JoinHandle<()>>,
    /// Optional setup function.
    setup: Option<S>,
}

impl ActorSystem {
    /// Create a new actor system with the default configuration.
    #[allow(clippy::new_without_default)]
    pub fn new() -> ActorSystem<!> {
        ActorSystem {
            threads: 1,
            sync_actors: Vec::new(),
            setup: None,
        }
    }

    /// Add a setup function.
    ///
    /// This function will be run on each worker thread the actor system
    /// creates. Only a single setup function can be added to the actor system.
    pub fn with_setup<F, E>(self, setup: F) -> ActorSystem<F>
    where
        F: FnOnce(ActorSystemRef) -> Result<(), E> + Send + Clone + 'static,
        E: Send,
    {
        ActorSystem {
            threads: self.threads,
            sync_actors: self.sync_actors,
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

    /// Spawn an synchronous actor that runs on its own thread.
    ///
    /// For more information and examples about synchronous actors see the
    /// [`actor::sync`] module.
    ///
    /// [`actor::sync`]: crate::actor::sync
    pub fn spawn_sync_actor<Sv, A, E, Arg, M>(
        &mut self,
        supervisor: Sv,
        actor: A,
        arg: Arg,
    ) -> Result<ActorRef<Sync<M>>, RuntimeError>
    where
        Sv: Supervisor<E, Arg> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        let (sender, receiver) = channel::unbounded();
        let actor_ref = ActorRef::new_sync(sender);

        debug!("creating thread for synchronous actor");
        let handle = thread::Builder::new()
            .name("heph_sync_actor".to_owned())
            .spawn(move || run_sync_actor(supervisor, actor, arg, receiver))
            .map_err(RuntimeError::start_thread)?;
        self.sync_actors.push(handle);

        Ok(actor_ref)
    }
}

impl<S> ActorSystem<S>
where
    S: SetupFn,
{
    /// Run the system.
    ///
    /// This will spawn a number of worker threads (see
    /// [`ActorSystem::num_threads`]) to run the system.
    pub fn run(self) -> Result<(), RuntimeError<S::Error>> {
        debug!("running actor system: worker_threads={}", self.threads);
        // Start our worker threads.
        let handles = (0..self.threads)
            .map(|id| {
                let setup = self.setup.clone();
                thread::Builder::new()
                    .name(format!("heph_worker{}", id))
                    .spawn(move || run_system(setup))
                    .map_err(RuntimeError::start_thread)
            })
            .collect::<Result<Vec<thread::JoinHandle<_>>, _>>()?;

        // TODO: handle errors better. Currently as long as the first thread
        // keeps running and all other threads crash we'll keep running. Not an
        // ideal situation.
        for handle in handles {
            handle.join().map_err(map_panic)
                .and_then(|res| res)?; // Result<Result<(), E>> -> Result<(), E>.
        }

        trace!("worker threads completed, waiting for synchronous actors");
        for handle in self.sync_actors {
            handle.join().map_err(map_panic)?;
        }

        Ok(())
    }
}

/// Maps a boxed panic messages to a `RuntimeError`.
fn map_panic<E>(err: Box<dyn Any + Send + 'static>) -> RuntimeError<E> {
    let msg = match err.downcast_ref::<&'static str>() {
        Some(s) => (*s).to_owned(),
        None => match err.downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => "unkown panic message".to_owned(),
        },
    };
    RuntimeError::panic(msg)
}

impl<S> fmt::Debug for ActorSystem<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorSystem")
            .field("threads", &self.threads)
            .field("sync_actors (length)", &self.sync_actors.len())
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
    where
        F: FnOnce(ActorSystemRef) -> Result<(), E> + Send + Clone + 'static,
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
where
    S: SetupFn,
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

/// Run a synchronous actor.
fn run_sync_actor<S, E, Arg, A, M>(mut supervisor: S, actor: A, mut arg: Arg, inbox: Receiver<M>)
where
    S: Supervisor<E, Arg>,
    A: SyncActor<Message = M, Argument = Arg, Error = E>,
{
    trace!("running synchronous actor");
    let mut ctx_data = SyncContextData::new(inbox);
    loop {
        let ctx = unsafe {
            // This is safe because the context data doesn't outlive the pointer
            // and the pointer is not null.
            SyncContext::new(NonNull::new_unchecked(&mut ctx_data))
        };
        match actor.run(ctx, arg) {
            Ok(()) => break,
            Err(err) => match supervisor.decide(err) {
                SupervisorStrategy::Restart(new_arg) => {
                    trace!("restarting synchronous actor");
                    arg = new_arg
                }
                SupervisorStrategy::Stop => break,
            },
        }
    }
    trace!("stopping synchronous actor");
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
    /// Event source for `Waker` events.
    waker_events: WakerEvents,
}

/// A wrapper around `crossbeam_channel::Receiver` that implements
/// `gaeaevent::Source`.
#[derive(Debug)]
struct WakerEvents {
    /// Receiving side of the channel for `Waker` events.
    receiver: Receiver<ProcessId>,
}

impl<ES, E> event::Source<ES, E> for WakerEvents
where
    ES: event::Sink,
{
    fn max_timeout(&self) -> Option<Duration> {
        if !self.receiver.is_empty() {
            Some(Duration::from_millis(0))
        } else {
            None
        }
    }

    fn poll(&mut self, event_sink: &mut ES) -> Result<(), E> {
        event_sink.extend(self.receiver.try_iter()
            .map(|pid| Event::new(pid.into(), Ready::READABLE)));
        Ok(())
    }
}

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Id used for the awakener.
const AWAKENER_ID: event::Id = event::Id(usize::max_value());

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new<E>() -> Result<RunningActorSystem, RuntimeError<E>> {
        // System queue for event notifications.
        let mut os_queue = OsQueue::new().map_err(RuntimeError::poll)?;
        let awakener = Awakener::new(&mut os_queue, AWAKENER_ID)
            .map_err(RuntimeError::poll)?;

        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = channel::unbounded();
        let waker_id = init_waker(awakener, waker_sender);

        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();

        // Internals of the running actor system.
        let internal = ActorSystemInternal {
            waker_id,
            scheduler_ref: RefCell::new(scheduler_ref),
            os_queue: RefCell::new(os_queue),
            timers: RefCell::new(Timers::new()),
            queue: RefCell::new(Queue::new()),
        };

        // The actor system we're going to run.
        Ok(RunningActorSystem {
            internal: Rc::new(internal),
            scheduler,
            waker_events: WakerEvents { receiver: waker_recv },
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

        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there is nothing to poll, as there would be no
            // processes to run then either.
            for _ in 0..RUN_POLL_RATIO {
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

            self.schedule_processes()?;
        }
    }

    /// Schedule processes.
    ///
    /// This polls the system os_queue and the waker notifications and schedules
    /// the processes notified.
    fn schedule_processes<E>(&mut self) -> Result<(), RuntimeError<E>> {
        trace!("polling event sources to schedule processes");

        // If there are any processes to run we don't want to block.
        let timeout = if self.scheduler.has_active_process() {
            Some(Duration::from_millis(0))
        } else {
            None
        };

        let mut os_queue = self.internal.os_queue.borrow_mut();
        let mut queue = self.internal.queue.borrow_mut();
        let mut timers = self.internal.timers.borrow_mut();
        let waker = &mut self.waker_events;
        poll(&mut [os_queue.deref_mut(), queue.deref_mut(), timers.deref_mut(), waker], &mut self.scheduler, timeout)
            .map_err(RuntimeError::poll)?;

        Ok(())
    }
}

/// A reference to an [`ActorSystem`].
///
/// A reference to the running actor system can be accessed via the actor
/// [`actor::Context::system_ref`].
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
    pub fn try_spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<Local<NA::Message>>, NA::Error>
    where
        S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
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
    pub fn spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<Local<NA::Message>>
    where
        S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
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
    #[allow(clippy::type_complexity)] // Not part of the public API, so it's OK.
    pub(crate) fn try_spawn_setup<S, NA, ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        mut new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<Local<NA::Message>>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
        NA: NewActor + 'static,
        ArgFn: FnOnce(ProcessId, &mut ActorSystemRef) -> Result<NA::Argument, ArgFnE>,
        NA::Actor: 'static,
    {
        let ActorSystemInternal {
            waker_id,
            ref scheduler_ref,
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
        let actor_ref = ActorRef::new_local(mailbox.downgrade());
        let ctx = actor::Context::new(pid, system_ref, mailbox.clone());
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        if options.schedule {
            new_waker(*waker_id, pid).wake()
        }

        // Add the actor to the scheduler.
        process_entry.add_actor(options.priority, supervisor, new_actor, actor, mailbox);

        Ok(actor_ref)
    }

    /// Register an `Evented` handle, see `OsQueue.register`.
    pub(crate) fn register<E>(&mut self, handle: &mut E, id: event::Id, interests: Interests, opt: RegisterOption) -> io::Result<()>
        where E: Evented + ?Sized,
    {
        self.internal.os_queue.borrow_mut().register(handle, id, interests, opt)
    }

    /// Reregister an `Evented` handle, see `OsQueue.reregister`.
    pub(crate) fn reregister<E>(&mut self, handle: &mut E, id: event::Id, interests: Interests, opt: RegisterOption) -> io::Result<()>
        where E: Evented + ?Sized,
    {
        self.internal.os_queue.borrow_mut().reregister(handle, id, interests, opt)
    }

    /// Get a clone of the sending end of the notification channel.
    pub(crate) fn new_waker(&self, pid: ProcessId) -> Waker {
        new_waker(self.internal.waker_id, pid)
    }

    /// Add a deadline to the event sources.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.internal.timers.borrow_mut().add_deadline(pid.into(), deadline);
    }

    pub(crate) fn notify(&mut self, pid: ProcessId) {
        self.internal.queue.borrow_mut().add(Event::new(pid.into(), Ready::READABLE));
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
    /// System queue, used for event notifications to support non-blocking I/O.
    os_queue: RefCell<OsQueue>,
    /// Timers, deadlines and timeouts.
    timers: RefCell<Timers>,
    /// User space queue.
    queue: RefCell<Queue>,
}
