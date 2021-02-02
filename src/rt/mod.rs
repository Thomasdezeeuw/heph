//! Module with Heph's runtime and related types.
//!
//! The module has two main types and a submodule:
//!
//! - [`Runtime`] is Heph's runtime, used to run all actors.
//! - [`RuntimeRef`] is a reference to a running runtime, used for example to
//!   spawn new actors.
//!
//! See [`Runtime`] for documentation on how to run an actor. For more examples
//! see the examples directory in the source code.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use std::{fmt, io, task, thread};

use log::{debug, trace};
use mio::{event, Interest, Poll, Registry, Token};

use crate::actor::context::{ThreadLocal, ThreadSafe};
use crate::actor::sync::SyncActor;
use crate::actor::{self, AddActorError, NewActor, PrivateSpawn, Spawn};
use crate::actor_ref::{ActorGroup, ActorRef};
use crate::supervisor::{Supervisor, SyncSupervisor};

mod coordinator;
mod error;
mod hack;
mod process;
mod signal;
mod timers;

pub(crate) mod access;
pub(crate) mod waker;

// Modules that are public (in the crate) when testing and private otherwise.
macro_rules! cfg_pub_test {
    ($( $mod_name: ident ),+) => {
        $(
            #[cfg(not(any(test, feature = "test")))]
            mod $mod_name;
            #[cfg(any(test, feature = "test"))]
            pub(crate) mod $mod_name;
        )+
    };
}

// Modules used in testing.
cfg_pub_test!(channel, scheduler, worker, sync_worker);

pub(crate) use access::PrivateAccess;
pub(crate) use process::ProcessId;
pub(crate) use timers::Timers; // Needed by the `test` module.
pub(crate) use waker::Waker;

pub mod options;
pub mod trace;

pub use access::Access;
pub use error::Error;
pub use options::{ActorOptions, SyncActorOptions};
pub use signal::Signal;

use coordinator::Coordinator;
use hack::SetupFn;
use scheduler::{LocalScheduler, SchedulerRef};
use sync_worker::SyncWorker;
use waker::{WakerId, MAX_THREADS};
use worker::Worker;

pub(crate) const SYNC_WORKER_ID_START: usize = 10000;
pub(crate) const SYNC_WORKER_ID_END: usize = SYNC_WORKER_ID_START + 10000;

#[test]
#[allow(clippy::assertions_on_constants)] // This is the point of the test.
fn sync_worker_id() {
    // Sync worker and worker thread ids may not overlap.
    assert!(SYNC_WORKER_ID_START > MAX_THREADS + 1); // Worker ids start at 1.
}

/// The runtime that runs all actors.
///
/// This type implements a builder pattern to build and start a runtime. It has
/// a single generic parameter `S` which is the type of the setup function. The
/// setup function is optional, which is represented by `!` (the never type).
///
/// The runtime will start workers threads that will run all actors, these
/// threads will run until all actors have returned.
///
/// ## Usage
///
/// Building an runtime starts with calling [`new`], this will create a new
/// runtime without a setup function, using a single worker thread.
///
/// An optional setup function can be added by calling [`with_setup`]. This
/// setup function will be called on each worker thread the runtime starts, and
/// thus has to be [`Clone`] and [`Send`]. This will change the `S` parameter
/// from `!` to an actual type (that of the provided setup function). Only a
/// single setup function can be used per runtime.
///
/// Now that the generic parameter is optionally defined we also have some more
/// configuration options. One such option is the number of threads the runtime
/// uses, this can configured with the [`num_threads`] method, or
/// [`use_all_cores`].
///
/// It is also possible to already start thread-safe actors using [`Spawn`]
/// implementation on `Runtime` , or synchronous actors using
/// [`spawn_sync_actor`]. Note that most actors should run as thread-*local*
/// actors however. To spawn such an actor see the [`Spawn`] implementation for
/// [`RuntimeRef`], which can spawn both thread-safe and thread-local actors.
/// For the different kind of actors see the [`actor`] module.
///
/// Finally after setting all the configuration options the runtime can be
/// [`start`]ed. This will spawn a number of worker threads to actually run the
/// actors.
///
/// [`new`]: Runtime::new
/// [`with_setup`]: Runtime::with_setup
/// [`num_threads`]: Runtime::num_threads
/// [`use_all_cores`]: Runtime::use_all_cores
/// [`spawn_sync_actor`]: Runtime::spawn_sync_actor
/// [`start`]: Runtime::start
///
/// ## Examples
///
/// This simple example shows how to run an `Runtime` and add how start a
/// single actor on each thread. This should print "Hello World" twice (once on
/// each worker thread started).
///
/// ```
/// #![feature(never_type)]
///
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
///
/// fn main() -> Result<(), rt::Error> {
///     // Build a new `Runtime`.
///     Runtime::new()?
///         // Start two worker threads.
///         .num_threads(2)
///         // On each worker thread run our setup function.
///         .with_setup(setup)
///         // And start the runtime.
///         .start()
/// }
///
/// // This setup function will on run on each created thread. In the case of
/// // this example we create two threads (see `main`).
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     // Asynchronous function don't yet implement the required `NewActor`
///     // trait, but function pointers do so we cast our type to a function
///     // pointer.
///     let actor = actor as fn(_, _) -> _;
///     // Each actors needs to be supervised, but since our actor doesn't
///     // return an error (the error type is `!`) we can get away with our
///     // `NoSupervisor` which doesn't do anything (as it can never be called).
///     let supervisor = NoSupervisor;
///     // All actors start with one or more arguments, in our case it's message
///     // used to greet people with. See `actor` below.
///     let arg = "Hello";
///     // Spawn the actor to run on our actor runtime. We get an actor reference
///     // back which can be used to send the actor messages, see below.
///     let actor_ref = runtime_ref.spawn_local(supervisor, actor, arg, ActorOptions::default());
///
///     // Send a message to the actor we just spawned.
///     actor_ref.try_send("World").unwrap();
///
///     Ok(())
/// }
///
/// /// Our actor that greets people.
/// async fn actor(mut ctx: actor::Context<&'static str>, msg: &'static str) -> Result<(), !> {
///     // `msg` is the argument passed to `spawn` in the `setup` function
///     // above, in this example it was "Hello".
///
///     // Using the context we can receive messages send to this actor, so here
///     // we'll receive the "World" message we send in the `setup` function.
///     if let Ok(name) = ctx.receive_next().await {
///         // This should print "Hello world"!
///         println!("{} {}", msg, name);
///     }
///     Ok(())
/// }
/// ```
pub struct Runtime<S = !> {
    /// Coordinator thread data.
    coordinator: Coordinator,
    /// Internals shared between the coordinator and the worker threads.
    shared: Arc<SharedRuntimeInternal>,
    /// Number of worker threads to create.
    threads: usize,
    /// Whether or not to automatically set CPU affinity.
    auto_cpu_affinity: bool,
    /// Synchronous actor thread handles.
    sync_actors: Vec<SyncWorker>,
    /// Optional setup function.
    setup: Option<S>,
    /// List of actor references that want to receive process signals.
    signals: ActorGroup<Signal>,
    /// Trace log.
    trace_log: Option<trace::Log>,
}

impl Runtime {
    /// Create a `Runtime` with the default configuration.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Result<Runtime<!>, Error> {
        let (coordinator, shared) = Coordinator::init().map_err(Error::coordinator)?;
        Ok(Runtime {
            coordinator,
            shared,
            threads: 1,
            auto_cpu_affinity: false,
            sync_actors: Vec::new(),
            setup: None,
            signals: ActorGroup::empty(),
            trace_log: None,
        })
    }

    /// Add a setup function.
    ///
    /// This function will be run on each worker thread the runtime creates.
    /// Only a single setup function can be added to the runtime.
    pub fn with_setup<F, E>(self, setup: F) -> Runtime<F>
    where
        F: FnOnce(RuntimeRef) -> Result<(), E> + Send + Clone + 'static,
        E: Send,
    {
        Runtime {
            coordinator: self.coordinator,
            shared: self.shared,
            threads: self.threads,
            auto_cpu_affinity: self.auto_cpu_affinity,
            sync_actors: self.sync_actors,
            setup: Some(setup),
            signals: self.signals,
            trace_log: self.trace_log,
        }
    }
}

impl<S> Runtime<S> {
    /// Set the number of worker threads to use, defaults to one.
    ///
    /// Most applications would want to use [`Runtime::use_all_cores`] which
    /// sets the number of threads equal to the number of CPU cores.
    pub fn num_threads(mut self, n: usize) -> Self {
        if n > MAX_THREADS {
            panic!(
                "Can't create {} worker threads, {} is the maximum",
                n, MAX_THREADS
            );
        } else if n == 0 {
            panic!("Can't create zero worker threads, one is the minimum");
        }
        self.threads = n;
        self
    }

    /// Set the number of worker threads equal to the number of CPU cores.
    ///
    /// See [`Runtime::num_threads`].
    pub fn use_all_cores(self) -> Self {
        let n = thread::available_concurrency()
            .map(|n| n.get())
            .unwrap_or(1);
        self.num_threads(n)
    }

    /// Returns the number of worker threads to use.
    ///
    /// See [`Runtime::num_threads`].
    pub fn get_threads(&self) -> usize {
        self.threads
    }

    /// Automatically set CPU affinity.
    ///
    /// The is mostly useful when using [`Runtime::use_all_cores`] to create a
    /// single worker thread per CPU core.
    pub fn auto_cpu_affinity(mut self) -> Self {
        self.auto_cpu_affinity = true;
        self
    }

    /// Generate a trace of the runtime, writing it to the file specified by
    /// `path`.
    ///
    /// See the [`mod@trace`] module for more information.
    ///
    /// # Notes
    ///
    /// To enable tracing of synchronous actors this must be called before
    /// calling [`spawn_sync_actor`].
    ///
    /// [`spawn_sync_actor`]: Runtime::spawn_sync_actor
    pub fn enable_tracing<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        match trace::Log::open(path.as_ref(), coordinator::TRACE_ID) {
            Ok(trace_log) => {
                self.trace_log = Some(trace_log);
                Ok(())
            }
            Err(err) => Err(Error::setup_trace(err)),
        }
    }

    /// Attempt to spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn try_spawn<Sv, NA>(
        &mut self,
        supervisor: Sv,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        Sv: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn spawn<Sv, NA>(
        &mut self,
        supervisor: Sv,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        Sv: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Error = !, Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn an synchronous actor that runs on its own thread.
    ///
    /// For more information and examples of synchronous actors see the
    /// [`actor::sync`] module.
    ///
    /// [`actor::sync`]: crate::actor::sync
    pub fn spawn_sync_actor<Sv, A, E, Arg, M>(
        &mut self,
        supervisor: Sv,
        actor: A,
        arg: Arg,
        options: SyncActorOptions,
    ) -> Result<ActorRef<M>, Error>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        let id = SYNC_WORKER_ID_START + self.sync_actors.len();
        // TODO: add actor's name to this.
        // This doesn't work with `actor::name` because the sync actors are
        // often converted into function pointers (using `as fn(_) -> _`) before
        // calling this. This results in a name which is something like
        // `fn(SyncContext<_>, ...) -> ...`, which is useless.
        // NOTE: a sync actor function (not pointer) *does* work with
        // `actor::name`.
        debug!("spawning synchronous actor: pid={}", id);

        let trace_log = if let Some(trace_log) = &self.trace_log {
            let trace_log = trace_log
                .new_stream(id as u32)
                .map_err(Error::start_sync_actor)?;
            Some(trace_log)
        } else {
            None
        };
        SyncWorker::start(id, supervisor, actor, arg, options, trace_log)
            .map(|(worker, actor_ref)| {
                self.sync_actors.push(worker);
                actor_ref
            })
            .map_err(Error::start_sync_actor)
    }

    /// Receive [process signals] as messages.
    ///
    /// This adds the `actor_ref` to the list of actor references that will
    /// receive a process signal.
    ///
    /// [process signals]: Signal
    pub fn receive_signals(&mut self, actor_ref: ActorRef<Signal>) {
        self.signals.add(actor_ref);
    }
}

impl<S> Runtime<S>
where
    S: SetupFn,
{
    /// Run the runtime.
    ///
    /// This will spawn a number of worker threads (see
    /// [`Runtime::num_threads`]) to run. After spawning all worker threads it
    /// will wait until all worker threads have returned, which happens when all
    /// actors have returned.
    ///
    /// In addition to waiting for all worker threads it will also watch for
    /// all process signals in [`Signal`] and relay them to actors that want to
    /// handle them, see the [`Signal`] type for more information.
    pub fn start(mut self) -> Result<(), Error<S::Error>> {
        debug!(
            "starting Heph runtime: worker_threads={}, sync_actors={}",
            self.threads,
            self.sync_actors.len()
        );

        // Start our worker threads.
        let timing = trace::start(&self.trace_log);
        let handles = (1..=self.threads)
            .map(|id| {
                let id = NonZeroUsize::new(id).unwrap();
                let setup = self.setup.clone();
                let trace_log = if let Some(trace_log) = &self.trace_log {
                    Some(trace_log.new_stream(id.get() as u32)?)
                } else {
                    None
                };
                Worker::start(
                    id,
                    setup,
                    self.shared.clone(),
                    self.auto_cpu_affinity,
                    trace_log,
                )
            })
            .collect::<io::Result<Vec<Worker<S::Error>>>>()
            .map_err(Error::start_worker)?;
        trace::finish(
            &mut self.trace_log,
            timing,
            "Creating worker threads",
            &[("amount", &self.threads)],
        );

        // Drop stuff we don't need anymore. For the setup function this is
        // extra important if it contains e.g. actor references.
        drop(self.setup);
        drop(self.shared);

        self.coordinator
            .run(handles, self.sync_actors, self.signals, self.trace_log)
    }
}

impl<S, Sv, NA> Spawn<Sv, NA, ThreadSafe> for Runtime<S>
where
    Sv: Send + Sync,
    NA: NewActor<Context = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
}

impl<S, Sv, NA> PrivateSpawn<Sv, NA, ThreadSafe> for Runtime<S>
where
    Sv: Send + Sync,
    NA: NewActor<Context = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: Sv,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        Sv: Supervisor<NA> + 'static,
        NA: NewActor<Context = ThreadSafe> + 'static,
        NA::Actor: 'static,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, ArgFnE>,
    {
        self.shared
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl<S> fmt::Debug for Runtime<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let setup = if self.setup.is_some() {
            &"Some"
        } else {
            &"None"
        };
        f.debug_struct("Runtime")
            .field("threads", &self.threads)
            .field("sync_actors (length)", &self.sync_actors.len())
            .field("setup", setup)
            .field("auto_cpu_affinity", &self.auto_cpu_affinity)
            .field("signals", &self.signals)
            .field("trace_log", &self.trace_log)
            .finish()
    }
}

/// A reference to a [`Runtime`].
///
/// This reference refers to the thread-local runtime, and thus can't be shared
/// across thread bounds. To share this reference (within the same thread) it
/// can be cloned.
///
/// This is passed to the [setup function] when starting the runtime and can be
/// accessed inside an actor by using the [`actor::Context::runtime`] method.
///
/// [setup function]: Runtime::with_setup
#[derive(Clone, Debug)]
pub struct RuntimeRef {
    /// A shared reference to the runtime's internals.
    internal: Rc<RuntimeInternal>,
}

impl RuntimeRef {
    /// Attempt to spawn a new thead-local actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn try_spawn_local<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Context = ThreadLocal> + 'static,
        NA::Actor: 'static,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thead-local actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn spawn_local<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Error = !, Context = ThreadLocal> + 'static,
        NA::Actor: 'static,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Attempt to spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn try_spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Error = !, Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Receive [process signals] as messages.
    ///
    /// This adds the `actor_ref` to the list of actor references that will
    /// receive a process signal.
    ///
    /// [process signals]: Signal
    pub fn receive_signals(&mut self, actor_ref: ActorRef<Signal>) {
        self.internal.signal_receivers.borrow_mut().push(actor_ref)
    }

    /// Register an `event::Source`, see [`mio::Registry::register`].
    pub(crate) fn register<S>(
        &mut self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.internal
            .poll
            .borrow()
            .registry()
            .register(source, token, interest)
    }

    /// Reregister an `event::Source`, see [`mio::Registry::reregister`].
    pub(crate) fn reregister<S>(
        &mut self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.internal
            .poll
            .borrow()
            .registry()
            .reregister(source, token, interest)
    }

    /// Create a new `Waker` implementation.
    pub(crate) fn new_waker(&self, pid: ProcessId) -> Waker {
        Waker::new(self.internal.waker_id, pid)
    }

    /// Get a clone of the sending end of the notification channel.
    ///
    /// # Notes
    ///
    /// Prefer `new_waker` if possible, only use `task::Waker` for `Future`s.
    pub(crate) fn new_local_task_waker(&self, pid: ProcessId) -> task::Waker {
        waker::new(self.internal.waker_id, pid)
    }

    /// Same as [`RuntimeRef::new_local_task_waker`] but provides a waker
    /// implementation for thread-safe actors.
    pub(crate) fn new_shared_task_waker(&self, pid: ProcessId) -> task::Waker {
        self.internal.shared.new_task_waker(pid)
    }

    /// Add a deadline to the event sources.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        trace!("adding deadline: pid={}, deadline={:?}", pid, deadline);
        self.internal
            .timers
            .borrow_mut()
            .add_deadline(pid, deadline);
    }

    /// Returns a copy of the shared internals.
    pub(crate) fn clone_shared(&self) -> Arc<SharedRuntimeInternal> {
        self.internal.shared.clone()
    }

    pub(crate) fn cpu(&self) -> Option<usize> {
        self.internal.cpu
    }
}

impl<S, NA> Spawn<S, NA, ThreadLocal> for RuntimeRef {}

impl<S, NA> PrivateSpawn<S, NA, ThreadLocal> for RuntimeRef {
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        mut new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Context = ThreadLocal> + 'static,
        NA::Actor: 'static,
        ArgFn:
            FnOnce(&mut actor::Context<NA::Message, ThreadLocal>) -> Result<NA::Argument, ArgFnE>,
    {
        // Setup adding a new process to the scheduler.
        let mut scheduler = self.internal.scheduler.borrow_mut();
        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let name = actor::name::<NA::Actor>();
        debug!("spawning thread-local actor: pid={}, name={}", pid, name);

        // Create our actor context and our actor with it.
        let (manager, sender, receiver) = inbox::Manager::new_small_channel();
        let actor_ref = ActorRef::local(sender);
        let mut ctx = actor::Context::new_local(pid, receiver, self.clone());
        // Create our actor argument, running any setup required by the caller.
        let arg = arg_fn(&mut ctx).map_err(AddActorError::ArgFn)?;
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        // Add the actor to the scheduler.
        actor_entry.add(
            options.priority(),
            supervisor,
            new_actor,
            actor,
            manager,
            options.is_ready(),
        );

        Ok(actor_ref)
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for RuntimeRef
where
    S: Send + Sync,
    NA: NewActor<Context = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
}

impl<S, NA> PrivateSpawn<S, NA, ThreadSafe> for RuntimeRef
where
    S: Send + Sync,
    NA: NewActor<Context = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Context = ThreadSafe> + 'static,
        NA::Actor: 'static,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, ArgFnE>,
    {
        self.internal
            .shared
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
struct RuntimeInternal {
    /// Runtime internals shared between threads, owned by the `Coordinator`.
    shared: Arc<SharedRuntimeInternal>,
    /// Waker id used to create a `Waker` for thread-local actors.
    waker_id: WakerId,
    /// Scheduler for thread-local actors.
    scheduler: RefCell<LocalScheduler>,
    /// OS poll, used for event notifications to support non-blocking I/O.
    poll: RefCell<Poll>,
    /// Timers, deadlines and timeouts.
    timers: RefCell<Timers>,
    /// Actor references to relay received `Signal`s to.
    signal_receivers: RefCell<Vec<ActorRef<Signal>>>,
    /// CPU the worker thread is bound to, or `None` if not set.
    cpu: Option<usize>,
}

/// Shared internals of the runtime.
#[derive(Debug)]
pub(crate) struct SharedRuntimeInternal {
    /// Waker id used to create a `Waker` for thread-safe actors.
    waker_id: WakerId,
    /// Waker used to wake the `Coordinator`, but not schedule any particular
    /// process.
    waker: Waker,
    /// Scheduler for thread-safe actors.
    scheduler: SchedulerRef,
    /// Registry for the `Coordinator`'s `Poll` instance.
    registry: Registry,
    // FIXME: `Timers` is not up to this job.
    timers: Arc<Mutex<Timers>>,
}

impl SharedRuntimeInternal {
    pub(crate) fn new(
        waker_id: WakerId,
        scheduler: SchedulerRef,
        registry: Registry,
        timers: Arc<Mutex<Timers>>,
    ) -> Arc<SharedRuntimeInternal> {
        let waker = Waker::new(waker_id, coordinator::WAKER.into());
        Arc::new(SharedRuntimeInternal {
            waker_id,
            waker,
            scheduler,
            registry,
            timers,
        })
    }

    /// Returns a new [`task::Waker`] for the thread-safe actor with `pid`.
    pub(crate) fn new_task_waker(&self, pid: ProcessId) -> task::Waker {
        waker::new(self.waker_id, pid)
    }

    pub(crate) fn new_waker(&self, pid: ProcessId) -> Waker {
        Waker::new(self.waker_id, pid)
    }

    /// Register an `event::Source`, see [`mio::Registry::register`].
    pub(crate) fn register<S>(
        &self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.registry.register(source, token, interest)
    }

    /// Reregister an `event::Source`, see [`mio::Registry::reregister`].
    pub(crate) fn reregister<S>(
        &self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.registry.reregister(source, token, interest)
    }

    pub(crate) fn add_deadline(&self, pid: ProcessId, deadline: Instant) {
        self.timers.lock().unwrap().add_deadline(pid, deadline);
        // Ensure that the coordinator isn't polling and miss the deadline.
        self.wake();
    }

    /// Wake the [`Coordinator`].
    pub(crate) fn wake(&self) {
        self.waker.wake()
    }

    pub(crate) fn spawn_setup<S, NA, ArgFn, ArgFnE>(
        self: &Arc<Self>,
        supervisor: S,
        mut new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = ThreadSafe> + Sync + Send + 'static,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, ArgFnE>,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        // Setup adding a new process to the scheduler.
        let actor_entry = self.scheduler.add_actor();
        let pid = actor_entry.pid();
        let name = actor::name::<NA::Actor>();
        debug!("spawning thread-safe actor: pid={}, name={}", pid, name);

        // Create our actor context and our actor with it.
        let (manager, sender, receiver) = inbox::Manager::new_small_channel();
        let actor_ref = ActorRef::local(sender);
        let mut ctx = actor::Context::new_shared(pid, receiver, self.clone());
        let arg = arg_fn(&mut ctx).map_err(AddActorError::ArgFn)?;
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        // Add the actor to the scheduler.
        actor_entry.add(
            options.priority(),
            supervisor,
            new_actor,
            actor,
            manager,
            options.is_ready(),
        );

        Ok(actor_ref)
    }
}
