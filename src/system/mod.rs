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

use std::any::Any;
use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::ptr::NonNull;
use std::rc::Rc;
use std::task::Waker;
use std::time::{Duration, Instant};
use std::{fmt, thread};

use crossbeam_channel::{self as channel, Receiver};
use log::{debug, trace};
use mio::{event, Events, Interests, Poll, Token};
use mio_pipe::new_pipe;
use mio_signals::{SignalSet, Signals};

use crate::actor::sync::{SyncActor, SyncContext, SyncContextData};
use crate::actor_ref::{ActorRef, Local, LocalTryMap, Sync, TryMap};
use crate::inbox::Inbox;
use crate::supervisor::{Supervisor, SupervisorStrategy, SyncSupervisor};
use crate::{actor, NewActor};

mod error;
mod process;
mod queue;
mod scheduler;
mod timers;
mod waker;

pub(crate) use process::ProcessId;

pub mod options;

pub use error::RuntimeError;
pub use options::ActorOptions;

use queue::Queue;
use scheduler::{Scheduler, SchedulerRef};
use timers::Timers;
use waker::{init_waker, new_waker, WakerId, MAX_THREADS};

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
    /// Unique id for sync actors.
    sync_actor_id: usize,
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
            sync_actor_id: SYNC_WORKER_ID_START,
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
            sync_actor_id: self.sync_actor_id,
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
    ) -> Result<ActorRef<Sync<M>>, RuntimeError>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        self.sync_actor_id += 1;
        SyncWorker::start(self.sync_actor_id, supervisor, actor, arg)
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

        cmain(handles, self.sync_actors)
    }
}

struct Worker<E> {
    id: usize,
    handle: thread::JoinHandle<Result<(), RuntimeError<E>>>,
    sender: mio_pipe::Sender,
    pending: Vec<Signal>,
}

impl<E> Worker<E> {
    /// Start a new worker thread.
    fn start<S>(id: usize, setup: Option<S>) -> io::Result<Worker<S::Error>>
    where
        S: SetupFn<Error = E>,
        E: Send + 'static,
    {
        new_pipe().and_then(|(sender, receiver)| {
            thread::Builder::new()
                .name(format!("heph_worker{}", id))
                .spawn(move || wmain(setup, receiver))
                .map(|handle| Worker {
                    id,
                    handle,
                    sender,
                    pending: Vec::new(),
                })
        })
    }

    /// Send the worker thread a `signal`.
    ///
    /// If the signal can't be send now it will be added the to the list of
    /// pending signals, which can be send using `send_pending_signals`.
    fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        let byte = signal.to_byte();
        loop {
            match self.sender.write(&[byte]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(_) => return Ok(()),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    // Can't write right now, we'll do so later.
                    self.pending.push(signal);
                    return Ok(());
                }
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }

    /// Send the worker thread any pending signals.
    fn send_pending_signals(&mut self) -> io::Result<()> {
        while let Some(signal) = self.pending.first().copied() {
            self.send_signal(signal)?;
            let _ = self.pending.remove(0);
        }
        Ok(())
    }
}

struct SyncWorker {
    id: usize,
    handle: thread::JoinHandle<()>,
    sender: mio_pipe::Sender,
    // None if the sync actor can't handle signals.
    signals: Option<ActorRef<TryMap<Signal>>>,
}

impl SyncWorker {
    /// Start a new thread that runs a sync actor.
    pub fn start<Sv, A, E, Arg, M>(
        id: usize,
        supervisor: Sv,
        actor: A,
        arg: Arg,
    ) -> io::Result<(SyncWorker, ActorRef<Sync<M>>)>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
        ActorRef<Sync<M>>: IntoSignalActorRef,
    {
        new_pipe().and_then(|(sender, receiver)| {
            let (send, inbox) = channel::unbounded();
            let actor_ref = ActorRef::new_sync(send);
            thread::Builder::new()
                .name(format!("heph_sync_actor{}", id))
                .spawn(move || smain(supervisor, actor, arg, inbox, receiver))
                .map(|handle| SyncWorker {
                    id,
                    handle,
                    sender,
                    signals: IntoSignalActorRef::into(&actor_ref),
                })
                .map(|worker| (worker, actor_ref))
        })
    }

    /// Send the sync actor thread a `signal`.
    fn send_signal(&mut self, signal: Signal) {
        if let Some(ref mut signals) = self.signals {
            let _ = signals.send(signal);
        }
    }
}

const SIGNAL: Token = Token(usize::max_value());

// NOTE: `workers` must be sorted based on `id`.
fn cmain<E>(
    mut workers: Vec<Worker<E>>,
    mut sync_workers: Vec<SyncWorker>,
) -> Result<(), RuntimeError<E>> {
    let mut poll = Poll::new().map_err(RuntimeError::coordinator)?;
    let mut events = Events::with_capacity(16);

    let mut signals = Signals::new(SignalSet::all()).map_err(RuntimeError::coordinator)?;
    poll.registry()
        .register(&signals, SIGNAL, Interests::READABLE)
        .map_err(RuntimeError::coordinator)?;

    for worker in workers.iter() {
        trace!("registering worker thread: {}", worker.id);
        poll.registry()
            .register(&worker.sender, Token(worker.id), Interests::WRITABLE)
            .map_err(RuntimeError::coordinator)?;
    }

    for sync_worker in sync_workers.iter() {
        trace!("registering sync actor worker thread: {}", sync_worker.id);
        poll.registry()
            .register(
                &sync_worker.sender,
                Token(sync_worker.id),
                Interests::WRITABLE,
            )
            .map_err(RuntimeError::coordinator)?;
    }

    loop {
        trace!("polling on coordinator");
        poll.poll(&mut events, None)
            .map_err(RuntimeError::coordinator)?;

        for event in events.iter() {
            match event.token() {
                SIGNAL => {
                    while let Some(signal) = signals.receive().map_err(RuntimeError::coordinator)? {
                        for worker in workers.iter_mut() {
                            worker
                                .send_signal(Signal::from_mio(signal))
                                .map_err(RuntimeError::coordinator)?;
                        }
                        for sync_worker in sync_workers.iter_mut() {
                            sync_worker.send_signal(Signal::from_mio(signal));
                        }
                    }
                }
                token => {
                    if token.0 >= SYNC_WORKER_ID_START {
                        if let Ok(i) = sync_workers.binary_search_by_key(&token.0, |w| w.id) {
                            if event.is_error() || event.is_hup() {
                                // Receiving end of the pipe is dropped, which means the
                                // worker has shut down.
                                let sync_worker = sync_workers.remove(i);
                                debug!("sync actor worker thread {} is done", sync_worker.id);

                                sync_worker.handle.join().map_err(map_panic)?;
                            }
                        }
                    } else if let Ok(i) = workers.binary_search_by_key(&token.0, |w| w.id) {
                        if event.is_error() || event.is_hup() {
                            // Receiving end of the pipe is dropped, which means the
                            // worker has shut down.
                            let worker = workers.remove(i);
                            debug!("worker thread {} is done", worker.id);

                            worker
                                .handle
                                .join()
                                .map_err(map_panic)
                                .and_then(|res| res)?;
                        } else if event.is_writable() {
                            workers[i]
                                .send_pending_signals()
                                .map_err(RuntimeError::coordinator)?;
                        }
                    }
                }
            }
        }

        if workers.is_empty() {
            break;
        }
    }

    Ok(())
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
    use std::marker;

    use crate::actor_ref::{ActorRef, Sync, TryMap};

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
        fn into(actor_ref: &Self) -> Option<ActorRef<TryMap<Signal>>>;
    }

    impl<M> IntoSignalActorRef for ActorRef<Sync<M>> {
        default fn into(_: &Self) -> Option<ActorRef<TryMap<Signal>>> {
            None
        }
    }

    impl<M> IntoSignalActorRef for ActorRef<Sync<M>>
    where
        M: TryFrom<Signal, Error = Signal> + Into<Signal> + marker::Sync + Send + 'static,
    {
        fn into(actor_ref: &Self) -> Option<ActorRef<TryMap<Signal>>> {
            Some(actor_ref.clone().try_map())
        }
    }
}

use hack::{IntoSignalActorRef, SetupFn};

/// Process signal.
///
/// Actors can receive signals by calling [`ActorSystemRef::receive_signals`]
/// with their actor reference.
///
/// Synchronous actors receive signals automatically if they implement the
/// required trait bounds, see [`SyncActor`] for more details.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Signal {
    /// Interrupt signal.
    ///
    /// This signal is received by the process when its controlling terminal
    /// wishes to interrupt the process. This signal will for example be send
    /// when Ctrl+C is pressed in most terminals.
    ///
    /// Corresponds to POSIX signal `SIGINT`.
    Interrupt,
    /// Termination request signal.
    ///
    /// This signal received when the process is requested to terminate. This
    /// allows the process to perform nice termination, releasing resources and
    /// saving state if appropriate. This signal will be send when using the
    /// `kill` command for example.
    ///
    /// Corresponds to POSIX signal `SIGTERM`.
    Terminate,
    /// Terminal quit signal.
    ///
    /// This signal is received when the process is requested to quit and
    /// perform a core dump.
    ///
    /// Corresponds to POSIX signal `SIGQUIT`.
    Quit,
}

impl Signal {
    /// Convert to a byte, parsing using [`from_byte`].
    ///
    /// [`from_byte`]: Signal::from_byte
    fn to_byte(self) -> u8 {
        use Signal::*;
        match self {
            Interrupt => 1,
            Terminate => 2,
            Quit => 3,
        }
    }

    /// Parse a signal converted to a byte using [`to_byte`].
    ///
    /// [`to_byte`]: Signal::to_byte
    fn from_byte(byte: u8) -> Option<Signal> {
        use Signal::*;
        match byte {
            1 => Some(Interrupt),
            2 => Some(Terminate),
            3 => Some(Quit),
            _ => None,
        }
    }

    /// Convert a `mio_signals::Signal` into our own `Signal`.
    fn from_mio(signal: mio_signals::Signal) -> Signal {
        match signal {
            mio_signals::Signal::Interrupt => Signal::Interrupt,
            mio_signals::Signal::Terminate => Signal::Terminate,
            mio_signals::Signal::Quit => Signal::Quit,
        }
    }
}

/// Run the actor system, with an optional `setup` function.
///
/// This is the entry point for the worker threads.
fn wmain<S>(setup: Option<S>, receiver: mio_pipe::Receiver) -> Result<(), RuntimeError<S::Error>>
where
    S: SetupFn,
{
    let actor_system = RunningActorSystem::new(receiver).map_err(RuntimeError::worker)?;

    // Run optional setup.
    if let Some(setup) = setup {
        let system_ref = actor_system.create_ref();
        setup.setup(system_ref).map_err(RuntimeError::setup)?;
    }

    // All setup is done, so we're ready to run the event loop.
    actor_system.run_event_loop()
}

/// Run a synchronous actor.
fn smain<S, E, Arg, A, M>(
    mut supervisor: S,
    actor: A,
    mut arg: Arg,
    inbox: Receiver<M>,
    receiver: mio_pipe::Receiver,
) where
    S: SyncSupervisor<A> + 'static,
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

    // Let the coordinator know we're done.
    drop(receiver);
}

/// The system that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct RunningActorSystem {
    /// Inside of the system, shared with zero or more `ActorSystemRef`s.
    internal: Rc<ActorSystemInternal>,
    events: Events,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
    /// Receiving end of the channel connected to the coordinator thread.
    receiver: mio_pipe::Receiver,
}

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Id used for the awakener.
const AWAKENER: Token = Token(usize::max_value());
const COORDINATOR: Token = Token(usize::max_value() - 1);

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new(receiver: mio_pipe::Receiver) -> io::Result<RunningActorSystem> {
        // System queue for event notifications.
        let poll = Poll::new()?;
        let awakener = mio::Waker::new(poll.registry(), AWAKENER)?;
        poll.registry()
            .register(&receiver, COORDINATOR, Interests::READABLE)?;

        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = channel::unbounded();
        let waker_id = init_waker(awakener, waker_sender);

        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();

        // Internals of the running actor system.
        let internal = ActorSystemInternal {
            waker_id,
            scheduler_ref: RefCell::new(scheduler_ref),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            queue: RefCell::new(Queue::new()),
            signal_receivers: RefCell::new(Vec::new()),
        };

        // The actor system we're going to run.
        Ok(RunningActorSystem {
            internal: Rc::new(internal),
            events: Events::with_capacity(128),
            scheduler,
            waker_events: waker_recv,
            receiver,
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
            trace!("running processes");
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

            self.schedule_processes().map_err(RuntimeError::worker)?;
        }
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> io::Result<()> {
        trace!("polling event sources to schedule processes");

        let timeout = self.determine_timeout();

        self.internal
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout)?;

        let mut poll_waker = false;
        let mut check_receiver = false;
        for event in self.events.iter() {
            match event.token() {
                AWAKENER => poll_waker = true,
                COORDINATOR => check_receiver = true,
                token => self.scheduler.schedule(token.into()),
            }
        }

        if check_receiver {
            self.receive_signals()?;
        }

        if poll_waker {
            // We must first mark the waker as polled and only after poll the
            // waker events to ensure we don't miss any wake ups.
            waker::mark_polled(self.internal.waker_id);

            trace!("polling wakup events");
            for pid in self.waker_events.try_iter() {
                self.scheduler.schedule(pid);
            }
        }

        trace!("polling user space events");
        for pid in self.internal.queue.borrow_mut().events() {
            self.scheduler.schedule(pid);
        }

        trace!("polling timers");
        for pid in self.internal.timers.borrow_mut().deadlines() {
            self.scheduler.schedule(pid);
        }

        Ok(())
    }

    fn determine_timeout(&self) -> Option<Duration> {
        // If there are any processes ready to run, any waker events or user
        // space events we don't want to block.
        if self.scheduler.has_active_process()
            || !self.waker_events.is_empty()
            || !self.internal.queue.borrow().is_empty()
        {
            Some(Duration::from_millis(0))
        } else if let Some(deadline) = self.internal.timers.borrow().next_deadline() {
            let now = Instant::now();
            if deadline <= now {
                // Deadline has already expired, so no blocking.
                Some(Duration::from_millis(0))
            } else {
                // Time between the deadline and right now.
                Some(deadline.duration_since(now))
            }
        } else {
            None
        }
    }

    fn receive_signals(&mut self) -> io::Result<()> {
        trace!("receiving process signals");
        loop {
            let mut buf = [0; 8];
            match self.receiver.read(&mut buf) {
                // Should never happen as this would mean the coordinator has
                // dropped the sending side.
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    let mut receivers = self.internal.signal_receivers.borrow_mut();
                    for signal in buf[..n].iter().copied().filter_map(Signal::from_byte) {
                        for receiver in receivers.iter_mut() {
                            // Don't care if we succeed in sending the message.
                            let _ = receiver.send(signal);
                        }
                    }
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }
}

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
    ) -> Result<ActorRef<Local<NA::Message>>, NA::Error>
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
    ) -> ActorRef<Local<NA::Message>>
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
    ) -> Result<ActorRef<Local<NA::Message>>, AddActorError<NA::Error, ArgFnE>>
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
        let actor_ref = ActorRef::new_local(inbox.create_ref());
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
    pub fn receive_signals(&mut self, actor_ref: ActorRef<LocalTryMap<Signal>>) {
        self.internal.signal_receivers.borrow_mut().push(actor_ref)
    }

    /// Register an `event::Source`, see `mio::Registry::register`.
    pub(crate) fn register<E>(
        &mut self,
        handle: &mut E,
        id: Token,
        interests: Interests,
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
        interests: Interests,
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
    signal_receivers: RefCell<Vec<ActorRef<LocalTryMap<Signal>>>>,
}
