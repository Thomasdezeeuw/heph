//! Specialised runtime for Heph actors.
//!
//! If this is your introduction to Heph it better to start with the [Heph
//! crate] first.
//!
//! The root of the crate has two main types:
//!
//! - [`Runtime`] is Heph's runtime, used to run all actors.
//! - [`RuntimeRef`] is a reference to a running runtime, used for example to
//!   spawn new actors.
//!
//! [Heph crate]: heph
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
//! thread-local actors. For more information on the different kind of actors
//! see the [`heph::actor`] documentation and the documentation in the [`spawn`]
//! module.
//!
//! To help with initialisation and spawning of thread-local actors it's possible
//! to run functions on all worker threads using [`run_on_workers`]. This will
//! run the same (cloned) function on all workers threads with access to a
//! [`RuntimeRef`].
//!
//! Finally after setting up the runtime and spawning actors the runtime can be
//! [`start`]ed, which runs all actors and waits for them to complete.
//!
//! For an example of all of the above, see below.
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
//! This example shows how to setup and run a [`Runtime`], adding a thread-local
//! actor on each thread. This should print four messages:
//!  * One from the synchronous actor.
//!  * One from the asynchronous thread-safe actor.
//!  * One from the asynchronous thread-local actor on each worker thread, thus
//!    two in total.
//!
//! ```
//! # #![feature(never_type)]
//! use heph::actor::{self, SyncContext};
//! use heph::supervisor::NoSupervisor;
//! use heph_rt::spawn::{ActorOptions, SyncActorOptions};
//! use heph_rt::{self as rt, Runtime, RuntimeRef};
//! use heph_rt::access::Sync;
//!
//! fn main() -> Result<(), rt::Error> {
//!     // Build a new `Runtime`.
//!     let mut runtime = Runtime::setup()
//!         // We can set the application name.
//!         .with_name("my_app".into())
//!         // Set the number of worker threads to two.
//!         .num_threads(2)
//!         // Finally we build the runtime.
//!         .build()?;
//!
//!     // Now the runtime is build we can start spawning our actors and
//!     // futures.
//!
//!     // We'll start with spawning a synchronous actor.
//!     // For more information on spawning actors see the spawn module.
//!     let actor_ref = runtime.spawn_sync_actor(NoSupervisor, sync_actor as fn(_) -> _, (), SyncActorOptions::default())?;
//!     // And sending it a message.
//!     actor_ref.try_send("Alice").unwrap();
//!
//!     // Next a thread-safe actor.
//!     let actor_ref = runtime.spawn(NoSupervisor, actor as fn(_, _) -> _, "thread-safe", ActorOptions::default());
//!     actor_ref.try_send("Bob").unwrap();
//!
//!     // To spawn thread-safe actors we need to run it on the worker thread,
//!     // which we can do using the `run_on_workers` function.
//!     // `run_on_workers` simply runs the provided function on each worker
//!     // thread.
//!     runtime.run_on_workers(setup)?;
//!
//!     // And once the setup is complete we can start the runtime.
//!     // This will run all the actors we spawned.
//!     runtime.start()
//! }
//!
//! // This setup function will on run on each created thread. In the case of
//! // this example we create two threads (see `main`).
//! fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
//!     // Spawn a thread-local actor.
//!     let actor_ref = runtime_ref.spawn_local(NoSupervisor, actor as fn(_, _) -> _, "thread-local", ActorOptions::default());
//!     actor_ref.try_send("Charlie").unwrap();
//!     Ok(())
//! }
//!
//! /// Our synchronous actor.
//! fn sync_actor(mut ctx: SyncContext<&'static str, Sync>) {
//!     if let Ok(name) = ctx.receive_next() {
//!         println!("Hello {name} from sync actor");
//!     }
//! }
//!
//! /// Our asynchronous actor that can be run as a thread-local or thread-safe actor.
//! async fn actor<RT>(mut ctx: actor::Context<&'static str, RT>, actor_kind: &'static str) {
//!     if let Ok(name) = ctx.receive_next().await {
//!         println!("Hello {name} from {actor_kind} actor");
//!     }
//! }
//! ```
//!
//! For more examples see the [examples directory] in the source code.
//!
//! [examples directory]: https://github.com/Thomasdezeeuw/heph/tree/main/rt/examples
//!
//! ## Features
//!
//! This crate has one optional: `test`. The `test` feature will enable the
//! `test` module which adds testing facilities.

#![feature(
    async_iterator,
    const_option,
    const_weak_new,
    doc_auto_cfg,
    doc_cfg_hide,
    drain_filter,
    impl_trait_in_assoc_type,
    io_slice_advance,
    is_sorted,
    maybe_uninit_array_assume_init,
    maybe_uninit_uninit_array,
    never_type,
    new_uninit,
    ptr_metadata,
    stmt_expr_attributes,
    waker_getters
)]
#![warn(
    anonymous_parameters,
    bare_trait_objects,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    variant_size_differences
)]
// Disallow warnings when running tests.
#![cfg_attr(test, deny(warnings))]
// Disallow warnings in examples, we want to set a good example after all.
#![doc(test(attr(deny(warnings))))]
// The `cfg(any(test, feature = "test"))` attribute creates a doc element
// staying that it's only supporting "using test or test", that is a bit
// confusing. So we hide those parts and instead manually replace all of them
// with: `doc(cfg(feature = "test"))`. That will stay it's only supported using
// the test feature.
#![doc(cfg_hide(any(test, feature = "test")))]

#[cfg(not(target_os = "linux"))]
compile_error!("Heph currently only supports Linux.");
#[cfg(not(target_pointer_width = "64"))]
compile_error!("Heph currently only supports 64 bit architectures.");

/// Helper macro to execute a system call that returns an `io::Result`.
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)? ) ) => {{
        let res = unsafe { libc::$fn($( $arg, )*) };
        if res == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
}

use std::convert::TryInto;
use std::future::Future;
use std::rc::Rc;
#[cfg(any(test, feature = "test"))]
use std::sync::Arc;
use std::task;
use std::time::{Duration, Instant};

use ::log::{as_debug, debug, warn};
use heph::actor::{ActorFuture, NewActor, SyncActor};
use heph::actor_ref::{ActorGroup, ActorRef};
use heph::supervisor::{Supervisor, SyncSupervisor};

pub mod access;
mod channel;
mod coordinator;
mod error;
pub mod io;
mod local;
pub mod log;
pub mod net;
pub mod pipe;
mod process;
mod scheduler;
mod setup;
mod shared;
mod signal;
pub mod spawn;
mod sync_worker;
#[cfg(target_os = "linux")]
pub mod systemd;
#[cfg(any(test, feature = "test"))]
pub mod test;
mod thread_waker;
pub mod timer;
mod timers;
pub mod trace;
#[doc(hidden)]
pub mod util;
mod wakers;
mod worker;

use process::ProcessId;

#[doc(no_inline)]
pub use access::{Access, Sync, ThreadLocal, ThreadSafe};
pub use error::Error;
pub use setup::Setup;
pub use signal::Signal;

use crate::process::{FutureProcess, Process};
use coordinator::Coordinator;
use local::waker::MAX_THREADS;
use spawn::{ActorOptions, FutureOptions, Spawn, SyncActorOptions};
use sync_worker::SyncWorker;
use timers::TimerToken;

const SYNC_WORKER_ID_START: usize = 10000;
const SYNC_WORKER_ID_END: usize = SYNC_WORKER_ID_START + 10000;

/// Returns `ptr` as `usize`.
const fn ptr_as_usize<T>(ptr: *const T) -> usize {
    union Pointer<T> {
        ptr: *const T,
        int: usize,
    }
    let ptr = Pointer { ptr };
    unsafe { ptr.int }
}

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
/// threads will run until all actors have returned. See the [crate
/// documentation] for more information.
///
/// [crate documentation]: crate
#[derive(Debug)]
pub struct Runtime {
    /// Coordinator thread data.
    coordinator: Coordinator,
    /// Worker threads.
    workers: Vec<worker::Handle>,
    /// Synchronous actor threads.
    sync_actors: Vec<SyncWorker>,
    /// List of actor references that want to receive process signals.
    signals: ActorGroup<Signal>,
    /// Trace log.
    trace_log: Option<trace::CoordinatorLog>,
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

    /// Attempt to spawn a new thread-safe actor.
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
        S: Supervisor<NA> + Send + std::marker::Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + std::marker::Sync + Send + 'static,
        NA::Actor: Send + std::marker::Sync + 'static,
        NA::Message: Send,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thread-safe actor.
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
        S: Supervisor<NA> + Send + std::marker::Sync + 'static,
        NA: NewActor<Error = !, RuntimeAccess = ThreadSafe> + std::marker::Sync + Send + 'static,
        NA::Actor: Send + std::marker::Sync + 'static,
        NA::Message: Send,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn an synchronous actor that runs on its own thread.
    ///
    /// For more information and examples of synchronous actors see the
    /// [`heph::actor`] module.
    pub fn spawn_sync_actor<S, A>(
        &mut self,
        supervisor: S,
        actor: A,
        arg: A::Argument,
        options: SyncActorOptions,
    ) -> Result<ActorRef<A::Message>, Error>
    where
        S: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<RuntimeAccess = Sync> + Send + 'static,
        A::Message: Send + 'static,
        A::Argument: Send + 'static,
    {
        let id = SYNC_WORKER_ID_START + self.sync_actors.len();
        if let Some(name) = options.name() {
            debug!(sync_worker_id = id, name = name; "spawning synchronous actor");
        } else {
            debug!(sync_worker_id = id; "spawning synchronous actor");
        }

        #[allow(clippy::cast_possible_truncation)]
        // Safety: MAX_THREADS always fits in u32.
        let trace_log = self
            .trace_log
            .as_ref()
            .map(|trace_log| trace_log.new_stream(id as u32));
        let shared = self.coordinator.shared_internals().clone();
        SyncWorker::start(id, supervisor, actor, arg, options, shared, trace_log)
            .map(|(worker, actor_ref)| {
                self.sync_actors.push(worker);
                actor_ref
            })
            .map_err(Error::start_sync_actor)
    }

    /// Spawn a thread-safe [`Future`].
    ///
    /// See [`RuntimeRef::spawn_future`] for more documentation.
    pub fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + std::marker::Sync + 'static,
    {
        self.coordinator
            .shared_internals()
            .spawn_future(future, options);
    }

    /// Run the function `f` on all worker threads.
    ///
    /// This can be used to spawn thread-local actors, or to initialise
    /// thread-local data on each worker thread ensuring that it's properly
    /// initialised without impacting the performance of the first request(s).
    pub fn run_on_workers<F, E>(&mut self, f: F) -> Result<(), Error>
    where
        F: FnOnce(RuntimeRef) -> Result<(), E> + Send + Clone + 'static,
        E: ToString,
    {
        debug!("sending user function to workers");
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
            workers = self.workers.len(), sync_actors = self.sync_actors.len();
            "starting Heph runtime"
        );
        self.coordinator
            .run(self.workers, self.sync_actors, self.signals, self.trace_log)
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for Runtime
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadSafe>,
    {
        self.coordinator
            .shared_internals()
            .try_spawn(supervisor, new_actor, arg, options)
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
    /// Attempt to spawn a new thread-local actor.
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
        NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
        NA::Actor: 'static,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thread-local actor.
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
        NA: NewActor<Error = !, RuntimeAccess = ThreadLocal> + 'static,
        NA::Actor: 'static,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Attempt to spawn a new thread-safe actor.
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
        S: Supervisor<NA> + Send + std::marker::Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + std::marker::Sync + Send + 'static,
        NA::Actor: Send + std::marker::Sync + 'static,
        NA::Message: Send,
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thread-safe actor.
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
        S: Supervisor<NA> + Send + std::marker::Sync + 'static,
        NA: NewActor<Error = !, RuntimeAccess = ThreadSafe> + std::marker::Sync + Send + 'static,
        NA::Actor: Send + std::marker::Sync + 'static,
        NA::Message: Send,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a thread-local [`Future`].
    ///
    /// Similar to thread-local actors this will only run on a single thread.
    /// See the discussion of thread-local vs. thread-safe actors in the
    /// [`spawn`] module for additional information.
    #[allow(clippy::needless_pass_by_value)]
    pub fn spawn_local_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + 'static,
    {
        _ = self
            .internals
            .scheduler
            .borrow_mut()
            .add_new_process(options.priority(), |pid| {
                let process = FutureProcess(future);
                let name = process.name();
                debug!(pid = pid.0, name = name; "spawning thread-local future");
                Ok::<_, !>((process, ()))
            });
    }

    /// Spawn a thread-safe [`Future`].
    ///
    /// Similar to thread-safe actors this can run on any of the workers
    /// threads. See the discussion of thread-local vs. thread-safe actors in
    /// the [`spawn`] module for additional information.
    pub fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + std::marker::Sync + 'static,
    {
        self.internals.shared.spawn_future(future, options);
    }

    /// Receive [process signals] as messages.
    ///
    /// This adds the `actor_ref` to the list of actor references that will
    /// receive a process signal.
    ///
    /// [process signals]: Signal
    pub fn receive_signals(&mut self, actor_ref: ActorRef<Signal>) {
        self.internals
            .signal_receivers
            .borrow_mut()
            .add_unique(actor_ref);
    }

    /// Get a clone of the sending end of the notification channel.
    ///
    /// # Notes
    ///
    /// Prefer `new_waker` if possible, only use `task::Waker` for `Future`s.
    #[cfg(any(test, feature = "test"))]
    fn new_local_task_waker(&self, pid: ProcessId) -> task::Waker {
        local::waker::new(self.internals.waker_id, pid)
    }

    /// Add a timer.
    pub(crate) fn add_timer(&self, deadline: Instant, waker: task::Waker) -> TimerToken {
        ::log::trace!(deadline = as_debug!(deadline); "adding timer");
        self.internals.timers.borrow_mut().add(deadline, waker)
    }

    /// Remove a previously set timer.
    pub(crate) fn remove_timer(&self, deadline: Instant, token: TimerToken) {
        ::log::trace!(deadline = as_debug!(deadline); "removing timer");
        self.internals.timers.borrow_mut().remove(deadline, token);
    }

    /// Returns a copy of the shared internals.
    #[cfg(any(test, feature = "test"))]
    fn clone_shared(&self) -> Arc<shared::RuntimeInternals> {
        self.internals.shared.clone()
    }

    fn cpu(&self) -> Option<usize> {
        self.internals.cpu
    }

    fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&*self.internals.trace_log.borrow())
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        pid: ProcessId,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish(
            (*self.internals.trace_log.borrow_mut()).as_mut(),
            timing,
            pid.0 as u64,
            description,
            attributes,
        );
    }
}

impl<S, NA> Spawn<S, NA, ThreadLocal> for RuntimeRef
where
    S: Supervisor<NA> + 'static,
    NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
    NA::Actor: 'static,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadLocal>,
    {
        self.internals
            .scheduler
            .borrow_mut()
            .add_new_process(options.priority(), |pid| {
                let name = NA::name();
                debug!(pid = pid.0, name = name; "spawning thread-local actor");
                let rt = ThreadLocal::new(pid, self.clone());
                ActorFuture::new(supervisor, new_actor, arg, rt)
            })
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for RuntimeRef
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadSafe>,
    {
        self.internals
            .shared
            .try_spawn(supervisor, new_actor, arg, options)
    }
}

fn cpu_usage(clock_id: libc::clockid_t) -> Duration {
    let mut duration = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(clock_id, &mut duration) } == -1 {
        let err = io::Error::last_os_error();
        warn!("error getting CPU time: {err}, using zero");
        Duration::ZERO
    } else {
        Duration::new(
            duration.tv_sec.try_into().unwrap_or(0),
            duration.tv_nsec.try_into().unwrap_or(u32::MAX),
        )
    }
}
