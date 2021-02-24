//! Module with Heph's runtime and related types.
//!
//! The two main types of this module are:
//!
//! - [`Runtime`] is Heph's runtime, used to run all actors.
//! - [`RuntimeRef`] is a reference to a running runtime, used for example to
//!   spawn new actors.
//!
//! ## Running Heph's runtime
//!
//! Building a runtime starts with calling [`setup`], which will create a new
//! [`Setup`](Setup) builder type, which allows configuration of the
//! [`Runtime`]. The [`new`] function can also be used, but is really only meant
//! for quick prototyping or testing.
//!
//! [`Setup`](Setup) has a number of configuration options. An example of one
//! such option is the number of threads the runtime uses, this can configured
//! with the [`num_threads`] and [`use_all_cores`] methods. When using
//! `use_all_cores` the CPU affinity can automatically be set using
//! [`auto_cpu_affinity`].
//!
//! Once the runtime is fully configured it can be [`build`], which returns the
//! [`Runtime`] type.
//!
//! After the runtime is build it is also possible to start thread-safe actors
//! using [`Spawn`] implementation on `Runtime` or [`try_spawn`]. Synchronous
//! actors can be spawned using [`spawn_sync_actor`]. Note however that most
//! actors should run as thread-*local* actors however. To spawn a thread-local
//! actor see the [`Spawn`] implementation for [`RuntimeRef`] or
//! [`RuntimeRef::try_spawn_local`], which can spawn both thread-safe and
//! thread-local actors. For documentation on the different kind of actors see
//! the [`actor`] module.
//!
//! To help with initialisation and spawning of thread-local actor it's possible
//! to run functions on all worker threads using [`run_on_workers`]. This will
//! run the same (cloned) function on all workers threads with access to a
//! [`RuntimeRef`].
//!
//! Finally after configurating the runtime and spawning actors the runtime can
//! be [`start`]ed, which runs all actors and waits for them to complete.
//!
//! [`setup`]: Runtime::setup
//! [`new`]: Runtime::new
//! [`num_threads`]: Setup::num_threads
//! [`use_all_cores`]: Setup::use_all_cores
//! [`auto_cpu_affinity`]: Setup::auto_cpu_affinity
//! [`build`]: Setup::build
//! [`try_spawn`]: Runtime::try_spawn
//! [`spawn_sync_actor`]: Runtime::spawn_sync_actor
//! [`run_on_workers`]: Runtime::run_on_workers
//! [`start`]: Runtime::start
//!
//! ## Examples
//!
//! This simple example shows how to run a `Runtime` and add how start a single
//! actor on each thread. This should print "Hello World" twice (once on each
//! worker thread started).
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
//!
//! fn main() -> Result<(), rt::Error> {
//!     // Build a new `Runtime` with two worker threads.
//!     let mut runtime = Runtime::setup().num_threads(2).build()?;
//!     // On each worker thread run our setup function.
//!     runtime.run_on_workers(setup)?;
//!     // And start the runtime.
//!     runtime.start()
//! }
//!
//! // This setup function will on run on each created thread. In the case of
//! // this example we create two threads (see `main`).
//! fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
//!     // Asynchronous function don't yet implement the required `NewActor`
//!     // trait, but function pointers do so we cast our type to a function
//!     // pointer.
//!     let actor = actor as fn(_, _) -> _;
//!     // Each actors needs to be supervised, but since our actor doesn't
//!     // return an error (the error type is `!`) we can get away with our
//!     // `NoSupervisor` which doesn't do anything (as it can never be called).
//!     let supervisor = NoSupervisor;
//!     // All actors start with one or more arguments, in our case it's message
//!     // used to greet people with. See `actor` below.
//!     let arg = "Hello";
//!     // Spawn the actor to run on our actor runtime. We get an actor reference
//!     // back which can be used to send the actor messages, see below.
//!     let actor_ref = runtime_ref.spawn_local(supervisor, actor, arg, ActorOptions::default());
//!
//!     // Send a message to the actor we just spawned.
//!     actor_ref.try_send("World").unwrap();
//!
//!     Ok(())
//! }
//!
//! /// Our actor that greets people.
//! async fn actor(mut ctx: actor::Context<&'static str>, msg: &'static str) {
//!     // `msg` is the argument passed to `spawn` in the `setup` function
//!     // above, in this example it was "Hello".
//!
//!     // Using the context we can receive messages send to this actor, so here
//!     // we'll receive the "World" message we send in the `setup` function.
//!     if let Ok(name) = ctx.receive_next().await {
//!         // This should print "Hello world"!
//!         println!("{} {}", msg, name);
//!     }
//! }
//! ```
//!
//! For more examples see the [examples directory] in the source code.
//!
//! [examples directory]: https://github.com/Thomasdezeeuw/heph/tree/master/examples

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use std::{io, task};

use log::{debug, trace};
use mio::{event, Interest, Token};

use crate::actor::{
    self, AddActorError, NewActor, PrivateSpawn, Spawn, SyncActor, ThreadLocal, ThreadSafe,
};
use crate::actor_ref::{ActorGroup, ActorRef};
use crate::supervisor::{Supervisor, SyncSupervisor};
use crate::trace;

mod coordinator;
mod error;
mod process;
mod setup;
mod signal;

pub(crate) mod access;
pub(crate) mod channel;
pub(crate) mod local;
pub(crate) mod shared;
pub(crate) mod sync_worker;
pub(crate) mod thread_waker;
pub(crate) mod waker;
pub(crate) mod worker;

pub mod options;

pub(crate) use access::PrivateAccess;
pub(crate) use process::ProcessId;

pub use access::Access;
pub use error::Error;
pub use options::{ActorOptions, SyncActorOptions};
pub use setup::Setup;
pub use signal::Signal;

use coordinator::Coordinator;
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

#[test]
#[allow(clippy::assertions_on_constants)] // This is the point of the test.
fn max_threads() {
    // `trace::Log` uses 32 bit integers as id.
    assert!(MAX_THREADS < u32::MAX as usize);
}

/// The runtime that runs all actors.
///
/// The runtime will start workers threads that will run all actors, these
/// threads will run until all actors have returned. See the [module]
/// documentation for more information.
///
/// [module]: crate::rt
#[derive(Debug)]
pub struct Runtime {
    /// Coordinator thread data.
    coordinator: Coordinator,
    /// Worker threads.
    workers: Vec<Worker>,
    /// Synchronous actor threads.
    sync_actors: Vec<SyncWorker>,
    /// List of actor references that want to receive process signals.
    signals: ActorGroup<Signal>,
    /// Trace log.
    trace_log: Option<trace::Log>,
}

impl Runtime {
    /// Setup a new `Runtime`.
    ///
    /// See [`Setup`] for the available configuration options.
    pub const fn setup() -> Setup {
        Setup::new()
    }

    /// Create a `Runtime` with the default configuration.
    ///
    /// # Notes
    ///
    /// This is mainly useful for quick prototyping and testing. When moving to
    /// production you'll likely want [setup] the runtime, at the very least to
    /// run a worker thread on all available CPU cores.
    ///
    /// [setup]: Runtime::setup
    #[allow(clippy::new_without_default)]
    pub fn new() -> Result<Runtime, Error> {
        Setup::new().build()
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

    /// Spawn an synchronous actor that runs on its own thread.
    ///
    /// For more information and examples of synchronous actors see the
    /// [`actor`] module.
    ///
    /// [`actor`]: crate::actor
    pub fn spawn_sync_actor<S, A>(
        &mut self,
        supervisor: S,
        actor: A,
        arg: A::Argument,
        options: SyncActorOptions,
    ) -> Result<ActorRef<A::Message>, Error>
    where
        S: SyncSupervisor<A> + Send + 'static,
        A: SyncActor + Send + 'static,
        A::Message: Send + 'static,
        A::Argument: Send + 'static,
    {
        let id = SYNC_WORKER_ID_START + self.sync_actors.len();
        match options.thread_name.as_ref() {
            Some(name) => debug!("spawning synchronous actor: pid={}, name='{}'", id, name),
            None => debug!("spawning synchronous actor: pid={}", id),
        }

        let trace_log = if let Some(trace_log) = &self.trace_log {
            #[allow(clippy::cast_possible_truncation)]
            let trace_log = trace_log
                // Safety: MAX_THREADS always fits in u32.
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

    /// Run the function `f` on all worker threads.
    ///
    /// This can be used to spawn thread-local actors, e.g. [`TcpServer`], or to
    /// initialise thread-local data on each worker thread ensuring that it's
    /// properly initialised without impacting the performance of the first
    /// request(s).
    ///
    /// [`TcpServer`]: crate::net::TcpServer
    pub fn run_on_workers<F, E>(&mut self, f: F) -> Result<(), Error>
    where
        F: FnOnce(RuntimeRef) -> Result<(), E> + Send + Clone + 'static,
        E: ToString,
    {
        for worker in &mut self.workers {
            let f = f.clone();
            let f = Box::new(move |runtime_ref| f(runtime_ref).map_err(|err| err.to_string()));
            worker
                .send_function(f)
                .map_err(|err| Error::coordinator(coordinator::Error::SendingFunc(err)))?;
        }
        Ok(())
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

    /// Run the runtime.
    ///
    /// This will wait until all spawned workers have finished, which happens
    /// when all actors have finished. In addition to waiting for all worker
    /// threads it will also watch for all process signals in [`Signal`] and
    /// relay them to actors that want to handle them, see the [`Signal`] type
    /// for more information.
    pub fn start(self) -> Result<(), Error> {
        debug!(
            "starting Heph runtime: sync_actors={}",
            self.sync_actors.len()
        );
        self.coordinator
            .run(self.workers, self.sync_actors, self.signals, self.trace_log)
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for Runtime
where
    S: Send + Sync,
    NA: NewActor<Context = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
}

impl<S, NA> PrivateSpawn<S, NA, ThreadSafe> for Runtime
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
        self.coordinator
            .shared_internals()
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

/// A reference to a [`Runtime`].
///
/// This reference refers to the thread-local runtime, and thus can't be shared
/// across thread bounds. To share this reference (within the same thread) it
/// can be cloned.
#[derive(Clone, Debug)]
pub struct RuntimeRef {
    /// A shared reference to the runtime's internals.
    internals: Rc<local::RuntimeInternals>,
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
        self.internals.signal_receivers.borrow_mut().push(actor_ref)
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
        self.internals
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
        self.internals
            .poll
            .borrow()
            .registry()
            .reregister(source, token, interest)
    }

    /// Get a clone of the sending end of the notification channel.
    ///
    /// # Notes
    ///
    /// Prefer `new_waker` if possible, only use `task::Waker` for `Future`s.
    pub(crate) fn new_local_task_waker(&self, pid: ProcessId) -> task::Waker {
        waker::new(self.internals.waker_id, pid)
    }

    /// Same as [`RuntimeRef::new_local_task_waker`] but provides a waker
    /// implementation for thread-safe actors.
    pub(crate) fn new_shared_task_waker(&self, pid: ProcessId) -> task::Waker {
        self.internals.shared.new_task_waker(pid)
    }

    /// Add a deadline to the event sources.
    ///
    /// This is used in the `timer` crate.
    pub(crate) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        trace!("adding deadline: pid={}, deadline={:?}", pid, deadline);
        self.internals.timers.borrow_mut().add(pid, deadline);
    }

    /// Returns a copy of the shared internals.
    pub(crate) fn clone_shared(&self) -> Arc<shared::RuntimeInternals> {
        self.internals.shared.clone()
    }

    pub(crate) fn cpu(&self) -> Option<usize> {
        self.internals.cpu
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
        let mut scheduler = self.internals.scheduler.borrow_mut();
        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let name = new_actor.name();
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
        self.internals
            .shared
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}
