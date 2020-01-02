//! Module with the actor system and related types.
//!
//! The module has two main types and a submodule:
//!
//! - [`ActorSystem`] is the actor system, used to run all actors.
//! - [`ActorSystemRef`] is a reference to a running actor system, used for
//!   example to spawn new actors.
//!
//! See [`ActorSystem`] for documentation on how to run an actor system. For
//! more examples see the examples directory in the source code.

use std::cell::RefCell;
use std::rc::Rc;
use std::task::Waker;
use std::time::Instant;
use std::{fmt, io};

use log::{debug, trace};
use mio::{event, Interest, Poll, Token};

use crate::actor::sync::SyncActor;
use crate::actor_ref::{ActorRef, LocalActorRef};
use crate::inbox::Inbox;
use crate::supervisor::{Supervisor, SyncSupervisor};
use crate::{actor, NewActor};

mod coordinator;
mod error;
mod process;
mod queue;
mod scheduler;
mod signal;
mod sync_worker;
mod timers;
mod waker;

// Used in test module.
pub(crate) mod worker;

pub(crate) use process::ProcessId;

pub mod options;

pub use error::RuntimeError;
pub use options::ActorOptions;
pub use signal::Signal;

use queue::Queue;
use scheduler::SchedulerRef;
use sync_worker::SyncWorker;
use timers::Timers;
use waker::{new_waker, WakerId, MAX_THREADS};
use worker::Worker;

const SYNC_WORKER_ID_START: usize = MAX_THREADS + 1;

/// The system that runs all actors.
///
/// This type implements a builder pattern to build and run an actor system. It
/// has a single generic parameter `S` which is the type of the setup function.
/// The setup function is optional, which is represented by `!` (the never
/// type).
///
/// The actor system will start workers threads that will run all actors, these
/// threads will run until all actors have returned.
///
/// ## Usage
///
/// Building an actor system starts with calling [`new`], this will create a new
/// system without a setup function, using a single worker thread.
///
/// An optional setup function can be added by calling [`with_setup`]. This
/// setup function will be called on each worker thread the actor system starts,
/// and thus has to be [`Clone`] and [`Send`]. This will change the `S`
/// parameter from `!` to an actual type (that of the provided setup function).
/// Only a single setup function can be used per actor system.
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
/// #![feature(never_type)]
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
/// // this example we create two threads (see `main`).
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
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
///     // Spawn the actor to run on our actor system. We get an actor reference
///     // back which can be used to send the actor messages, see below.
///     let mut actor_ref = system_ref.spawn(supervisor, actor, arg, ActorOptions::default());
///
///     // Send a message to the actor we just spawned.
///     actor_ref <<= "World";
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
///     let name = ctx.receive_next().await;
///     // This should print "Hello world"!
///     println!("{} {}", msg, name);
///     Ok(())
/// }
/// ```
pub struct ActorSystem<S = !> {
    /// Number of worker threads to create.
    threads: usize,
    /// Synchronous actor thread handles.
    sync_actors: Vec<SyncWorker>,
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
    /// Set the number of worker threads to use, defaults to one.
    ///
    /// Most applications would want to use [`ActorSystem::use_all_cores`] which
    /// sets the number of threads equal to the number of CPU cores.
    pub fn num_threads(mut self, n: usize) -> Self {
        if n > MAX_THREADS {
            panic!(
                "Can't create {} worker threads, {} is the maximum",
                n, MAX_THREADS
            );
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
    /// For more information and examples of synchronous actors see the
    /// [`actor::sync`] module.
    ///
    /// [`actor::sync`]: crate::actor::sync
    pub fn spawn_sync_actor<Sv, A, E, Arg, M>(
        &mut self,
        supervisor: Sv,
        actor: A,
        arg: Arg,
    ) -> Result<ActorRef<M>, RuntimeError>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        let id = SYNC_WORKER_ID_START + self.sync_actors.len();
        SyncWorker::start(id, supervisor, actor, arg)
            .map(|(worker, actor_ref)| {
                self.sync_actors.push(worker);
                actor_ref
            })
            .map_err(RuntimeError::start_thread)
    }
}

impl<S> ActorSystem<S>
where
    S: SetupFn,
{
    /// Run the system.
    ///
    /// This will spawn a number of worker threads (see
    /// [`ActorSystem::num_threads`]) to run the system. After spawning all
    /// worker threads it will wait until all worker threads have returned,
    /// which happens when all actors have returned.
    ///
    /// In addition to waiting for all worker threads it will also watch for
    /// all process signals in [`Signal`] and relay them to actors that want to
    /// handle them, see the [`Signal`] type for more information.
    pub fn run(self) -> Result<(), RuntimeError<S::Error>> {
        debug!("running actor system: worker_threads={}", self.threads);
        // Start our worker threads.
        let handles = (0..self.threads)
            .map(|id| Worker::start(id, self.setup.clone()))
            .collect::<io::Result<Vec<Worker<S::Error>>>>()
            .map_err(RuntimeError::start_thread)?;

        coordinator::main(handles, self.sync_actors)
    }
}

impl<S> fmt::Debug for ActorSystem<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let setup = if self.setup.is_some() {
            &"Some"
        } else {
            &"None"
        };
        f.debug_struct("ActorSystem")
            .field("threads", &self.threads)
            .field("sync_actors (length)", &self.sync_actors.len())
            .field("setup", setup)
            .finish()
    }
}

/// Collection of hacks.
mod hack {
    use std::convert::TryFrom;

    use crate::actor_ref::ActorRef;

    use super::{ActorSystemRef, Signal};

    /// A hack to allow us to call `ActorSystem<!>.run`.
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

    /// Hack to allow `SyncWorker` to convert its actor reference to a reference
    /// that receives signals if possible, or returns none.
    pub trait IntoSignalActorRef {
        fn into(actor_ref: &Self) -> Option<ActorRef<Signal>>;
    }

    impl<M> IntoSignalActorRef for ActorRef<M> {
        default fn into(_: &Self) -> Option<ActorRef<Signal>> {
            None
        }
    }

    impl<M> IntoSignalActorRef for ActorRef<M>
    where
        M: TryFrom<Signal, Error = Signal> + Into<Signal> + Sync + Send + 'static,
    {
        fn into(actor_ref: &Self) -> Option<ActorRef<Signal>> {
            Some(actor_ref.clone().try_map())
        }
    }
}

use hack::SetupFn;

/// A reference to an [`ActorSystem`].
///
/// This reference refers to the thread-local actor system, and thus can't be
/// shared across thread bounds. To share this reference (within the same
/// thread) it can be cloned.
///
/// This is passed to the [setup function] when starting the actor system and
/// can be accessed inside an actor by using the [`actor::Context::system_ref`]
/// method.
///
/// [setup function]: ActorSystem::with_setup
#[derive(Clone, Debug)]
pub struct ActorSystemRef {
    /// A shared reference to the actor system's internals.
    internal: Rc<ActorSystemInternal>,
}

impl ActorSystemRef {
    /// Attempts to spawn an actor on the actor system.
    ///
    /// Arguments:
    /// * `supervisor`: all actors need supervision, the `supervisor` is the
    ///   supervisor for this actor, see the [`Supervisor`] trait for more
    ///   information.
    /// * `new_actor`: the [`NewActor`] implementation that defines how to start
    ///   the actor.
    /// * `arg`: the argument passed when starting the actor, and
    /// * `options`: the actor options used to spawn the new actors.
    ///
    /// See [`ActorSystem`] for an example on spawning actors.
    ///
    /// # Notes
    ///
    /// When using a [`NewActor`] implementation that never returns an error,
    /// such as the implementation provided by async function, it's easier to
    /// use the [`spawn`] method.
    ///
    /// [`spawn`]: ActorSystemRef::spawn
    pub fn try_spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<LocalActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + 'static,
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
    ) -> LocalActorRef<NA::Message>
    where
        S: Supervisor<NA> + 'static,
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
    ) -> Result<LocalActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
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
        let inbox = Inbox::new(pid, self.clone());
        let actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
        let ctx = actor::Context::new(pid, system_ref, inbox.clone());
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        if options.should_schedule() {
            new_waker(*waker_id, pid).wake()
        }

        // Add the actor to the scheduler.
        process_entry.add_actor(options.priority(), supervisor, new_actor, actor, inbox);

        Ok(actor_ref)
    }

    /// Receive [process signals] as messages.
    ///
    /// This adds the `actor_ref` to the list of actor references that will
    /// receive a process signal.
    ///
    /// [process signals]: Signal
    pub fn receive_signals(&mut self, actor_ref: LocalActorRef<Signal>) {
        self.internal.signal_receivers.borrow_mut().push(actor_ref)
    }

    /// Register an `event::Source`, see `mio::Registry::register`.
    pub(crate) fn register<E>(
        &mut self,
        handle: &mut E,
        id: Token,
        interests: Interest,
    ) -> io::Result<()>
    where
        E: event::Source + ?Sized,
    {
        self.internal
            .poll
            .borrow_mut()
            .registry()
            .register(handle, id, interests)
    }

    /// Reregister an `event::Source`, see `mio::Registry::reregister`.
    pub(crate) fn reregister<E>(
        &mut self,
        handle: &mut E,
        id: Token,
        interests: Interest,
    ) -> io::Result<()>
    where
        E: event::Source + ?Sized,
    {
        self.internal
            .poll
            .borrow_mut()
            .registry()
            .reregister(handle, id, interests)
    }

    /// Get a clone of the sending end of the notification channel.
    pub(crate) fn new_waker(&self, pid: ProcessId) -> Waker {
        new_waker(self.internal.waker_id, pid)
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

    /// Notify that process with `pid` is ready to run. Used by the inbox when
    /// it receives a message.
    pub(crate) fn notify(&mut self, pid: ProcessId) {
        trace!("notifying: pid={}", pid);
        self.internal.queue.borrow_mut().add(pid);
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
    poll: RefCell<Poll>,
    /// Timers, deadlines and timeouts.
    timers: RefCell<Timers>,
    /// User space queue.
    queue: RefCell<Queue>,
    signal_receivers: RefCell<Vec<LocalActorRef<Signal>>>,
}
