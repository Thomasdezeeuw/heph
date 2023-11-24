//! Worker thread code.
//!
//! A worker thread manages part of the [`Runtime`]. It manages two parts; the
//! local and shared (between workers) parts of the runtime. The local part
//! include thread-local actors and futures, timers for those local actors, I/O
//! state, etc. The can be found in [`Worker`]. The shared part is similar, but
//! not the sole responsibility of a single worker, all workers collectively are
//! responsible for it. This shared part can be fore in
//! [`shared::RuntimeInternals`].
//!
//! Creating a new worker starts with calling [`setup`] to prepare various
//! things that need to happen on the main/coordinator thread. After that worker
//! thread can be [started], which runs [`Worker::run`] in a new thread.
//!
//! [`Runtime`]: crate::Runtime
//! [started]: WorkerSetup::start

use std::cell::RefMut;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io, task, thread};

use crossbeam_channel::{self, Receiver};
use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use log::{as_debug, debug, trace};

use crate::error::StringError;
use crate::local::RuntimeInternals;
use crate::process::ProcessId;
use crate::setup::set_cpu_affinity;
use crate::spawn::options::ActorOptions;
use crate::wakers::Wakers;
use crate::{self as rt, shared, trace, RuntimeRef, Signal, ThreadLocal};

/// Number of system actors (spawned in the local scheduler).
pub(crate) const SYSTEM_ACTORS: usize = 1;

/// Number of processes to run in between calls to poll.
///
/// This number is chosen arbitrarily.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Target time for the duration of a single iteration of the event loop.
///
/// If the event loop iteration elapses this timeout no more processes are run,
/// regardless of how many have run so far.
// TODO: make this configurable.
const MAX_EVENT_LOOP_DURATION: Duration = Duration::from_millis(5);

/// Setup a new worker thread.
///
/// Use [`WorkerSetup::start`] to spawn the worker thread.
pub(crate) fn setup(
    id: NonZeroUsize,
    coordinator_sq: &a10::SubmissionQueue,
) -> io::Result<(WorkerSetup, a10::SubmissionQueue)> {
    let ring = a10::Ring::config(128)
        .attach_queue(coordinator_sq)
        .build()?;
    Ok(setup2(id, ring))
}

/// Test version of [`setup`].
#[cfg(any(test, feature = "test"))]
pub(crate) fn setup_test() -> io::Result<(WorkerSetup, a10::SubmissionQueue)> {
    let ring = a10::Ring::config(128).build()?;
    Ok(setup2(NonZeroUsize::MAX, ring))
}

/// Second part of the [`setup`].
fn setup2(id: NonZeroUsize, ring: a10::Ring) -> (WorkerSetup, a10::SubmissionQueue) {
    let sq = ring.submission_queue().clone();

    // Setup the waking mechanism.
    let (waker_sender, waker_events) = crossbeam_channel::unbounded();
    let wakers = Wakers::new(waker_sender, sq.clone());

    let setup = WorkerSetup {
        id,
        ring,
        wakers,
        waker_events,
    };
    (setup, sq)
}

/// Setup work required before starting a worker thread, see [`setup`].
pub(crate) struct WorkerSetup {
    /// See [`WorkerSetup::id`].
    id: NonZeroUsize,
    /// io_uring completion ring.
    ring: a10::Ring,
    /// Creation of `task::Waker`s for for thread-local actors.
    wakers: Wakers,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
}

impl WorkerSetup {
    /// Start a new worker thread.
    pub(crate) fn start(
        self,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> io::Result<Handle> {
        let id = self.id;
        self.start_named(
            shared_internals,
            auto_cpu_affinity,
            trace_log,
            format!("Worker {id}"),
        )
    }

    pub(crate) fn start_named(
        self,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
        thread_name: String,
    ) -> io::Result<Handle> {
        let sq = self.ring.submission_queue().clone();
        rt::channel::new(sq).and_then(move |(sender, receiver)| {
            let id = self.id;
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    let worker = Worker::setup(
                        self,
                        receiver,
                        shared_internals,
                        auto_cpu_affinity,
                        trace_log,
                    );
                    worker.run().map_err(rt::Error::worker)
                })
                .map(|handle| Handle {
                    id,
                    channel: sender,
                    handle,
                })
        })
    }

    /// Return the worker's id.
    pub(crate) const fn id(&self) -> usize {
        self.id.get()
    }
}

/// Handle to a worker thread.
#[derive(Debug)]
pub(crate) struct Handle {
    /// Unique id (among all threads in the [`rt::Runtime`]).
    id: NonZeroUsize,
    /// Two-way communication channel to share messages with the worker thread.
    channel: rt::channel::Sender<Control>,
    /// Handle for the actual thread.
    #[allow(clippy::struct_field_names)]
    handle: thread::JoinHandle<Result<(), rt::Error>>,
}

impl Handle {
    /// Return the worker's id.
    pub(crate) const fn id(&self) -> usize {
        self.id.get()
    }

    /// Send the worker thread a signal that the runtime has started.
    pub(crate) fn send_runtime_started(&self) -> io::Result<()> {
        self.channel.send(Control::Started)
    }

    /// Send the worker thread a `signal`.
    pub(crate) fn send_signal(&self, signal: Signal) -> io::Result<()> {
        self.channel.send(Control::Signal(signal))
    }

    /// Send the worker thread the function `f` to run.
    pub(crate) fn send_function(
        &self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> io::Result<()> {
        self.channel.send(Control::Run(f))
    }

    /// See [`thread::JoinHandle::is_finished`].
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// See [`thread::JoinHandle::join`].
    pub(crate) fn join(self) -> thread::Result<Result<(), rt::Error>> {
        self.handle.join()
    }
}

/// Worker that runs thread-local and thread-safe actors and futurers, and
/// holds and manages everything that is required to run them.
pub(crate) struct Worker {
    /// Internals of the runtime, shared with zero or more [`RuntimeRef`]s.
    internals: Rc<RuntimeInternals>,
    /// Receiving side of the channel for waker events, see the
    /// [`rt::local::waker`] module for the implementation.
    waker_events: Receiver<ProcessId>,
}

impl Worker {
    /// Setup the worker. Must be called on the worker thread.
    pub(crate) fn setup(
        setup: WorkerSetup,
        receiver: rt::channel::Receiver<Control>,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> Worker {
        let worker_id = setup.id.get();
        let timing = trace::start(&trace_log);

        let cpu = if auto_cpu_affinity {
            set_cpu_affinity(setup.id)
        } else {
            None
        };

        let internals = Rc::new(RuntimeInternals::new(
            setup.id,
            shared_internals,
            setup.wakers,
            setup.ring,
            cpu,
            trace_log,
        ));

        trace!(worker_id = worker_id; "spawning system actors");
        let runtime_ref = RuntimeRef {
            internals: internals.clone(),
        };
        spawn_system_actors(runtime_ref, receiver);

        let mut worker = Worker {
            internals,
            waker_events: setup.waker_events,
        };

        trace::finish_rt(
            worker.trace_log().as_mut(),
            timing,
            "Initialising the worker thread",
            &[],
        );
        worker
    }

    /// Run the worker.
    pub(crate) fn run(mut self) -> Result<(), Error> {
        debug!(worker_id = self.internals.id.get(); "starting worker");
        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            let mut n = 0;
            let mut elapsed = Duration::ZERO;
            while n < RUN_POLL_RATIO && elapsed < MAX_EVENT_LOOP_DURATION {
                match self.run_local_process() {
                    Some(process_elapsed) => {
                        n += 1;
                        elapsed += process_elapsed;
                    }
                    None => break,
                }
            }
            while n < RUN_POLL_RATIO && elapsed < MAX_EVENT_LOOP_DURATION {
                match self.run_shared_process() {
                    Some(process_elapsed) => {
                        n += 1;
                        elapsed += process_elapsed;
                    }
                    None => break,
                }
            }

            if let Some(err) = self.internals.take_err() {
                return Err(err);
            }
            if self.internals.started() && !self.has_user_process() {
                debug!(worker_id = self.internals.id.get(); "no processes to run, stopping worker");
                self.internals.shared.wake_all_workers();
                return Ok(());
            }

            self.schedule_processes()?;
        }
    }

    /// Attempts to run a single local process.
    ///
    /// Returns the duration for which the process ran, `None` if no process was
    /// ran.
    fn run_local_process(&mut self) -> Option<Duration> {
        let process = self.internals.scheduler.borrow_mut().next_process();
        match process {
            Some(mut process) => {
                let timing = trace::start(&*self.internals.trace_log.borrow());
                let pid = process.as_ref().id();
                let name = process.as_ref().name();
                debug!(worker_id = self.internals.id.get(), pid = pid.0, name = name; "running local process");
                // TODO: reuse wakers, maybe by storing them in the processes?
                let waker = self.internals.wakers.borrow_mut().new_task_waker(pid);
                let mut ctx = task::Context::from_waker(&waker);
                let result = process.as_mut().run(&mut ctx);
                match result.result {
                    task::Poll::Ready(()) => {
                        self.internals.scheduler.borrow_mut().complete(process);
                    }
                    task::Poll::Pending => {
                        self.internals
                            .scheduler
                            .borrow_mut()
                            .add_back_process(process);
                    }
                }
                trace::finish_rt(
                    self.internals.trace_log.borrow_mut().as_mut(),
                    timing,
                    "Running thread-local process",
                    &[("id", &pid.0), ("name", &name)],
                );
                Some(result.elapsed)
            }
            None => None,
        }
    }

    /// Attempts to run a single shared process.
    ///
    /// Returns the duration for which the process ran, `None` if no process was
    /// ran.
    fn run_shared_process(&mut self) -> Option<Duration> {
        let process = self.internals.shared.remove_process();
        match process {
            Some(mut process) => {
                let timing = trace::start(&*self.internals.trace_log.borrow());
                let pid = process.as_ref().id();
                let name = process.as_ref().name();
                debug!(worker_id = self.internals.id.get(), pid = pid.0, name = name; "running shared process");
                let waker = self.internals.shared.new_task_waker(pid);
                let mut ctx = task::Context::from_waker(&waker);
                let result = process.as_mut().run(&mut ctx);
                match result.result {
                    task::Poll::Ready(()) => {
                        self.internals.shared.complete(process);
                    }
                    task::Poll::Pending => {
                        self.internals.shared.add_back_process(process);
                    }
                }
                trace::finish_rt(
                    self.internals.trace_log.borrow_mut().as_mut(),
                    timing,
                    "Running thread-safe process",
                    &[("id", &pid.0), ("name", &name)],
                );
                Some(result.elapsed)
            }
            None => None,
        }
    }

    /// Returns `true` if there are processes in either the local or shared
    /// schedulers.
    fn has_user_process(&self) -> bool {
        self.internals.scheduler.borrow().has_user_process() || self.internals.shared.has_process()
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> Result<(), Error> {
        trace!(worker_id = self.internals.id.get(); "polling event sources to schedule processes");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        // Schedule local and shared processes based on various event sources.
        self.poll_os().map_err(Error::Polling)?;
        let mut local_amount = self.schedule_from_waker();
        let now = Instant::now();
        local_amount += self.schedule_from_local_timers(now);
        let shared_amount = self.schedule_from_shared_timers(now);

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling processes",
            &[
                ("local amount", &local_amount),
                ("shared amount", &shared_amount),
                ("total amount", &(local_amount + shared_amount)),
            ],
        );

        // Possibly wake other worker threads if we've scheduled any shared
        // processes (that we can't directly run).
        self.wake_workers(local_amount, shared_amount);

        Ok(())
    }

    /// Schedule processes based on user space waker events, e.g. used by the
    /// `Future` task system.
    fn schedule_from_waker(&mut self) -> usize {
        trace!(worker_id = self.internals.id.get(); "polling wakup events");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.waker_events.try_iter() {
            trace!(worker_id = self.internals.id.get(), pid = pid.0; "waking up local process");
            scheduler.mark_ready(pid);
            amount += 1;
        }

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-local processes based on wake-up events",
            &[("amount", &amount)],
        );
        amount
    }

    /// Schedule processes based on local timers.
    fn schedule_from_local_timers(&mut self, now: Instant) -> usize {
        trace!(worker_id = self.internals.id.get(); "polling local timers");
        let timing = trace::start(&*self.internals.trace_log.borrow());
        let amount = self.internals.timers.borrow_mut().expire_timers(now);
        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-local processes based on timers",
            &[("amount", &amount)],
        );
        amount
    }

    /// Schedule processes based on shared timers.
    fn schedule_from_shared_timers(&mut self, now: Instant) -> usize {
        trace!(worker_id = self.internals.id.get(); "polling shared timers");
        let timing = trace::start(&*self.internals.trace_log.borrow());
        let amount = self.internals.shared.expire_timers(now);
        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-safe processes based on timers",
            &[("amount", &amount)],
        );
        amount
    }

    /// Wake worker threads based on the amount of local scheduled processes
    /// (`local_amount`) and the amount of scheduled shared processes
    /// (`shared_amount`).
    fn wake_workers(&mut self, local_amount: usize, shared_amount: usize) {
        let wake_n = if local_amount == 0 {
            // We don't have to run any local processes, so we can run a shared
            // process ourselves.
            shared_amount.saturating_sub(1)
        } else {
            shared_amount
        };

        if wake_n != 0 {
            trace!(worker_id = self.internals.id.get(); "waking {wake_n} worker threads");
            let timing = trace::start(&*self.internals.trace_log.borrow());
            self.internals.shared.wake_workers(wake_n);
            trace::finish_rt(
                self.internals.trace_log.borrow_mut().as_mut(),
                timing,
                "Waking worker threads",
                &[("amount", &wake_n)],
            );
        }
    }

    /// Poll for OS events, filling `self.events`.
    ///
    /// Returns a boolean indicating if the shared timers should be checked.
    fn poll_os(&mut self) -> io::Result<()> {
        let timing = trace::start(&*self.internals.trace_log.borrow());

        // First process any shared completions, this influences
        // `determine_timeout` below as we might schedule shared processes etc.
        // Note that the call never blocks.
        trace!(worker_id = self.internals.id.get(); "polling shared ring");
        self.internals.shared.try_poll_ring()?;

        let timeout = self.determine_timeout();
        trace!(worker_id = self.internals.id.get(), timeout = as_debug!(timeout); "polling for OS events");
        self.internals.ring.borrow_mut().poll(timeout)?;

        // Since we could have been polling our own ring for a long time we poll
        // the shared ring again.
        trace!(worker_id = self.internals.id.get(); "polling shared ring");
        self.internals.shared.try_poll_ring()?;

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Polling for OS events",
            &[],
        );
        Ok(())
    }

    /// Determine the timeout to be used in polling.
    fn determine_timeout(&self) -> Option<Duration> {
        if self.internals.scheduler.borrow().has_ready_process()
            || !self.waker_events.is_empty()
            || self.internals.shared.has_ready_process()
        {
            // If there are any processes ready to run (local or shared), or any
            // waker events we don't want to block.
            return Some(Duration::ZERO);
        }

        let now = Instant::now();
        match self.internals.timers.borrow_mut().next() {
            Some(deadline) => match deadline.checked_duration_since(now) {
                // Deadline has already expired, so no blocking.
                None => Some(Duration::ZERO),
                // Check the shared timers with the current deadline.
                timeout @ Some(..) => self.internals.shared.next_timeout(now, timeout),
            },
            // If there are no local timers check the shared timers.
            None => self.internals.shared.next_timeout(now, None),
        }
    }

    /// Create a new reference to this runtime.
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn create_ref(&self) -> RuntimeRef {
        RuntimeRef {
            internals: self.internals.clone(),
        }
    }

    /// Returns the trace log, if any.
    fn trace_log(&mut self) -> RefMut<'_, Option<trace::Log>> {
        self.internals.trace_log.borrow_mut()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Wake the coordinator forcing it check if the workers are still alive.
        self.internals.shared.wake_coordinator();
    }
}

/// Error running a [`Worker`].
#[derive(Debug)]
pub(crate) enum Error {
    /// Error polling for OS events.
    Polling(io::Error),
    /// Process was interrupted (i.e. received process signal), but no actor can
    /// receive the signal.
    ProcessInterrupted,
    /// Error running user function.
    UserFunction(StringError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Polling(err) => write!(f, "error polling OS: {err}"),
            Error::ProcessInterrupted => write!(
                f,
                "received process signal, but no receivers for it: stopping runtime"
            ),
            Error::UserFunction(err) => write!(f, "error running user function: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Polling(ref err) => Some(err),
            Error::ProcessInterrupted => None,
            Error::UserFunction(ref err) => Some(err),
        }
    }
}

/// Spawn all system actors.
#[allow(clippy::assertions_on_constants)]
fn spawn_system_actors(mut runtime_ref: RuntimeRef, receiver: rt::channel::Receiver<Control>) {
    let _ = runtime_ref.spawn_local(
        NoSupervisor,
        actor_fn(comm_actor),
        receiver,
        ActorOptions::SYSTEM,
    );
    // Keep this up to date, otherwise we'll exit early.
    assert!(SYSTEM_ACTORS == 1);
}

/// Control message send to the worker threads by the coordinator, handled by
/// [`comm_actor`].
#[allow(variant_size_differences)] // Can't make `Run` smaller.
pub(crate) enum Control {
    /// Runtime has started, i.e. [`rt::Runtime::start`] was called.
    Started,
    /// Process received a signal.
    Signal(Signal),
    /// Run a user defined function.
    Run(Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Control::Started => f.write_str("Control::Started"),
            Control::Signal(signal) => f.debug_tuple("Control::Signal").field(&signal).finish(),
            Control::Run(..) => f.write_str("Control::Run(..)"),
        }
    }
}

/// System actor that communicates with the coordinator.
///
/// It receives the coordinator's messages and processes them.
async fn comm_actor(
    ctx: actor::Context<!, ThreadLocal>,
    mut receiver: rt::channel::Receiver<Control>,
) {
    while let Some(msg) = receiver.recv().await {
        let internals = &ctx.runtime_ref().internals;
        trace!(worker_id = internals.id.get(), message = as_debug!(msg); "processing coordinator message");
        let timing = trace::start(&*internals.trace_log.borrow());
        match msg {
            Control::Started => internals.start(),
            Control::Signal(signal) => internals.relay_signal(signal),
            Control::Run(f) => internals.run_user_function(f),
        }
        trace::finish_rt(
            internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Processing communication message(s)",
            &[],
        );
    }
}
