//! Worker thread code.
//!
//! A worker thread manages part of the [`Runtime`]. It manages two parts; the
//! local and shared (between workers) parts of the runtime. The local part
//! include thread-local actors and futures, timers for those local actors, I/O
//! state, etc. This can be found in [`Worker`]. The shared part is similar, but
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

use std::num::NonZeroUsize;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use std::{fmt, io, task, thread};

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, SendError};
use heph::panic_message;
use heph::supervisor::NoSupervisor;

use crate::error::StringError;
use crate::setup::scheduler::{Process, Scheduler, SchedulerProcess};
use crate::setup::timers::Timers;
use crate::spawn::options::ActorOptions;
use crate::trace::Trace;
use crate::util::next;
use crate::{self as rt, Access, RuntimeRef, ThreadLocal, local, process, shared, trace};

/// Number of system actors (spawned in the local scheduler).
const SYSTEM_ACTORS: usize = 2;

/// Configuratiob of a [`Worker`].
pub(crate) struct Conf<FS, FT> {
    pub(crate) id: NonZeroUsize,
    pub(crate) shared_internals: Arc<shared::RuntimeInternals>,
    pub(crate) create_scheduler: FS,
    pub(crate) create_timers: FT,
    pub(crate) auto_cpu_affinity: bool,
    pub(crate) run_poll_ratio: usize,
    pub(crate) max_run_time: Duration,
}

/// Spawn a new worker thread.
pub(crate) fn spawn_thread<FS, S, FT, T>(conf: Conf<FS, FT>) -> io::Result<Spawned>
where
    FS: FnOnce(a10::SubmissionQueue) -> S + Send + 'static,
    S: Scheduler + 'static,
    FT: FnOnce() -> T + Send + 'static,
    T: Timers + 'static,
{
    let id = conf.id;
    let sys_ref = Arc::new(OnceLock::new());
    let init = sys_ref.clone();
    let handle = thread::Builder::new()
        .name(format!("Worker {id}"))
        .spawn(move || main(conf, init))?;
    Ok(Spawned {
        id,
        sys_ref,
        handle,
    })
}

/// Worker thread that is spawned, but not fully running yet.
pub(crate) struct Spawned {
    id: NonZeroUsize,
    sys_ref: Arc<OnceLock<ActorRef<Control>>>,
    handle: thread::JoinHandle<Result<(), Error>>,
}

impl Spawned {
    /// Wait until the worker is running.
    pub(crate) fn wait_running(self) -> Handle {
        let sys_ref = self.sys_ref.wait().clone();
        Handle {
            id: self.id,
            sys_ref,
            handle: self.handle,
        }
    }
}

/// Handle to a worker thread.
#[derive(Debug)]
pub(crate) struct Handle {
    /// Unique id (among all threads in the [`rt::Runtime`]).
    id: NonZeroUsize,
    sys_ref: ActorRef<Control>,
    #[allow(clippy::struct_field_names)]
    handle: thread::JoinHandle<Result<(), Error>>,
}

impl Handle {
    /// Return the worker's id.
    pub(crate) const fn id(&self) -> NonZeroUsize {
        self.id
    }

    /// Send the worker thread a signal that the runtime has started.
    pub(crate) fn send_runtime_started(&self) -> Result<(), SendError> {
        self.sys_ref.try_send(Control::Started)
    }

    /// Send the worker thread a `signal`.
    pub(crate) fn send_signal(&self, signal: process::Signal) -> Result<(), SendError> {
        self.sys_ref.try_send(Control::Signal(signal))
    }

    /// Send the worker thread the function `f` to run.
    pub(crate) fn send_function(
        &self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> Result<(), SendError> {
        self.sys_ref.try_send(Control::Run(f))
    }

    #[cfg(any(test, feature = "test"))]
    pub(crate) async fn send_function_wait(
        &self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> Result<(), SendError> {
        self.sys_ref.send(Control::Run(f)).await
    }

    /// See [`thread::JoinHandle::join`].
    pub(crate) fn join(self) -> Result<(), rt::Error> {
        match self.handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(rt::Error::worker(err)),
            Err(err) => Err(rt::Error::worker_panic(err)),
        }
    }
}

/// Main function of a worker thread.
fn main<FS, S, FT, T>(
    conf: Conf<FS, FT>,
    init: Arc<OnceLock<ActorRef<Control>>>,
) -> Result<(), Error>
where
    FS: FnOnce(a10::SubmissionQueue) -> S,
    S: Scheduler + 'static,
    FT: FnOnce() -> T,
    T: Timers + 'static,
{
    let trace_log = conf.shared_internals.worker_trace_log(conf.id);

    let timing = trace::start(&trace_log);
    let worker = Worker::setup(conf, trace_log, init)?;
    trace::finish_rt(
        worker.internals.trace_log.borrow_mut().as_mut(),
        timing,
        "Initialised the worker thread",
        &[],
    );

    worker.run()
}

/// Worker that runs thread-local and thread-safe actors and futurers, and
/// holds and manages everything that is required to run them.
pub(crate) struct Worker<S, T> {
    /// Internals of the runtime, shared with zero or more [`RuntimeRef`]s.
    internals: Rc<local::RuntimeInternals<S, T>>,
    // See Setup options with the same name for documentation.
    run_poll_ratio: usize,
    max_run_time: Duration,
}

impl<S, T> Worker<S, T>
where
    S: Scheduler + 'static,
    T: Timers + 'static,
{
    /// Set up a worker.
    fn setup<FS, FT>(
        conf: Conf<FS, FT>,
        trace_log: Option<trace::Log>,
        init: Arc<OnceLock<ActorRef<Control>>>,
    ) -> Result<Worker<S, T>, Error>
    where
        FS: FnOnce(a10::SubmissionQueue) -> S,
        FT: FnOnce() -> T,
    {
        #[rustfmt::skip]
        let Conf { id, shared_internals, create_scheduler, create_timers, auto_cpu_affinity, run_poll_ratio, max_run_time } = conf;

        let config = a10::Ring::config();
        #[cfg(any(target_os = "android", target_os = "linux"))]
        let config = config
            .single_issuer()
            .defer_task_run()
            .attach_queue(shared_internals.sq());

        // Set CPU affinity on the thread itself and on the ring.
        #[allow(unused_mut)]
        let mut cpu_affinity: Option<usize> = None;
        #[cfg(any(target_os = "android", target_os = "linux"))]
        if auto_cpu_affinity {
            let cpu_id = id.get() - 1; // Worker ids start at 1, cpus at 0.
            let cpu_set = cpu_set(cpu_id);
            match set_affinity(&cpu_set) {
                Ok(()) => {
                    log::debug!(worker_id = id; "worker thread CPU affinity set to {cpu_id}");
                    cpu_affinity = Some(cpu_id);
                }
                Err(err) => {
                    log::warn!(worker_id = id; "failed to set CPU affinity on thread: {err}");
                }
            }
        }
        #[cfg(not(any(target_os = "android", target_os = "linux")))]
        let _ = auto_cpu_affinity; // Silence unused code warning.
        let ring = config.build().map_err(Error::Setup)?;

        // Finally we can create the runtime internals.
        let sq = ring.sq();
        let internals = Rc::new(local::RuntimeInternals::new(
            id,
            shared_internals,
            ring,
            create_scheduler(sq),
            create_timers(),
            cpu_affinity,
            trace_log,
        ));

        // Spawn our system actors.
        let runtime_ref = RuntimeRef {
            internals: internals.clone(),
        };
        let sys_ref = spawn_system_actors(runtime_ref);

        // Let the coordinator know we're ready to start.
        let res = init.set(sys_ref);
        assert!(res.is_ok());
        drop(init);

        Ok(Worker {
            internals,
            run_poll_ratio,
            max_run_time,
        })
    }

    /// Run the worker.
    pub(crate) fn run(mut self) -> Result<(), Error> {
        log::debug!(worker_id = self.internals.id; "starting worker");
        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            let mut n = 0;
            let mut elapsed = Duration::ZERO;
            while n < self.run_poll_ratio && elapsed < self.max_run_time {
                match self.run_local_process() {
                    Some(process_elapsed) => {
                        n += 1;
                        elapsed += process_elapsed;
                    }
                    None => break,
                }
            }
            while n < self.run_poll_ratio && elapsed < self.max_run_time {
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
                log::debug!(worker_id = self.internals.id; "no processes to run, stopping worker");
                return Ok(());
            }

            self.schedule_processes()?;
        }
    }

    /// Attempts to run a single local process.
    ///
    /// Returns the duration for which the process ran, `None` if no process was
    /// ran.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn run_local_process(&mut self) -> Option<Duration> {
        let process = self.internals.scheduler.borrow_mut().next_process();
        match process {
            Some((mut process, waker)) => {
                let timing = trace::start(&*self.internals.trace_log.borrow());
                let pid = process.id();
                let name = process.name();
                log::debug!(worker_id = self.internals.id, pid, name; "running local process");
                let mut ctx = task::Context::from_waker(&waker);
                let stats = Pin::new(&mut process).run(&mut ctx);
                match stats.result() {
                    task::Poll::Ready(()) => {
                        log::trace!(worker_id = self.internals.id, pid, name; "removing completed local process");
                        if let Err(err) = self
                            .internals
                            .scheduler
                            .borrow_mut()
                            .complete_process(process)
                        {
                            let msg = panic_message(&*err);
                            log::warn!(worker_id = self.internals.id, pid, name; "panicked while dropping process: {msg}");
                        }
                    }
                    task::Poll::Pending => {
                        log::trace!(worker_id = self.internals.id, pid, name; "adding back local process");
                        self.internals
                            .scheduler
                            .borrow_mut()
                            .add_back_process(process, stats);
                    }
                }
                trace::finish_rt(
                    self.internals.trace_log.borrow_mut().as_mut(),
                    timing,
                    "Running thread-local process",
                    &[("id", &pid.data()), ("name", &name)],
                );
                Some(stats.elapsed())
            }
            None => None,
        }
    }

    /// Attempts to run a single shared process.
    ///
    /// Returns the duration for which the process ran, `None` if no process was
    /// ran.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn run_shared_process(&mut self) -> Option<Duration> {
        let process = self.internals.shared.remove_process();
        match process {
            Some(mut process) => {
                let timing = trace::start(&*self.internals.trace_log.borrow());
                let pid = process.id();
                let name = process.name();
                log::debug!(worker_id = self.internals.id, pid, name; "running shared process");
                let waker = self.internals.shared.new_task_waker(pid);
                let mut ctx = task::Context::from_waker(&waker);
                let stats = process.as_mut().run(&mut ctx);
                match stats.result() {
                    task::Poll::Ready(()) => {
                        self.internals.shared.complete(process);
                    }
                    task::Poll::Pending => {
                        self.internals.shared.add_back_process(process, stats);
                    }
                }
                trace::finish_rt(
                    self.internals.trace_log.borrow_mut().as_mut(),
                    timing,
                    "Running thread-safe process",
                    &[("id", &pid.data()), ("name", &name)],
                );
                Some(stats.elapsed())
            }
            None => None,
        }
    }

    /// Returns `true` if there are processes in either the local or shared
    /// schedulers.
    fn has_user_process(&self) -> bool {
        let local_scheduler = self.internals.scheduler.borrow();
        if local_scheduler.has_process() {
            let mut processes = local_scheduler.processes_ready();
            if processes > SYSTEM_ACTORS {
                return true;
            }
            processes += local_scheduler.processes_inactive();
            if processes > SYSTEM_ACTORS {
                return true;
            }
        }
        self.internals.shared.has_process()
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> Result<(), Error> {
        let timing = trace::start(&*self.internals.trace_log.borrow());
        log::trace!(worker_id = self.internals.id; "scheduling processes");

        // Schedule local and shared processes based on various event sources.
        self.poll_os().map_err(Error::Polling)?;
        let now = Instant::now();
        let mut local_amount = self.schedule_from_local_timers(now);
        local_amount += self.schedule_local_processes();
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

        Ok(())
    }

    /// Schedule local processes based on user space waker events, e.g. used by
    /// the `Future` task system.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn schedule_local_processes(&mut self) -> usize {
        let timing = trace::start(&*self.internals.trace_log.borrow());
        log::trace!(worker_id = self.internals.id; "scheduling thread-local processes");

        let amount = self.internals.scheduler.borrow_mut().process_wakeups();

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-local processes",
            &[("amount", &amount)],
        );
        amount
    }

    /// Schedule processes based on local timers.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn schedule_from_local_timers(&mut self, now: Instant) -> usize {
        let timing = trace::start(&*self.internals.trace_log.borrow());
        log::trace!(worker_id = self.internals.id; "polling local timers");

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
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn schedule_from_shared_timers(&mut self, now: Instant) -> usize {
        let timing = trace::start(&*self.internals.trace_log.borrow());
        log::trace!(worker_id = self.internals.id; "polling shared timers");

        let amount = self.internals.shared.expire_timers(now);

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-safe processes based on timers",
            &[("amount", &amount)],
        );
        amount
    }

    /// Poll for OS events, filling `self.events`.
    ///
    /// Returns a boolean indicating if the shared timers should be checked.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn poll_os(&mut self) -> io::Result<()> {
        let timing = trace::start(&*self.internals.trace_log.borrow());

        // First submit any outstanding shared submissions for I/O and process
        // any shared completions as this influences determine_timeout below as
        // we might schedule shared processes etc.
        // Note: this call never blocks. poll_actor below will be awoken if the
        // ring needs to be polled.
        log::trace!(worker_id = self.internals.id; "polling shared ring");
        self.internals.shared.try_poll_ring()?;

        let timeout = self.determine_timeout();
        log::trace!(worker_id = self.internals.id, timeout:?; "polling for OS events");
        self.internals.ring.borrow_mut().poll(timeout)?;

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
            || self.internals.shared.has_ready_process()
        {
            // If there are any processes ready to run (local or shared) we
            // don't want to block.
            return Some(Duration::ZERO);
        }

        let now = Instant::now();
        match self.internals.timers.borrow_mut().next_deadline() {
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
}

/// Create a cpu set that may only run on `cpu_id`.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn cpu_set(cpu_id: usize) -> libc::cpu_set_t {
    let mut cpu_set = unsafe { std::mem::zeroed() };
    unsafe { libc::CPU_ZERO(&mut cpu_set) };
    unsafe { libc::CPU_SET(cpu_id % libc::CPU_SETSIZE as usize, &mut cpu_set) };
    cpu_set
}

/// Set the affinity of this thread to the `cpu_set`.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn set_affinity(cpu_set: &libc::cpu_set_t) -> io::Result<()> {
    let thread = unsafe { libc::pthread_self() };
    let res = unsafe { libc::pthread_setaffinity_np(thread, size_of_val(cpu_set), cpu_set) };
    if res == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

impl<S, T> Drop for Worker<S, T> {
    fn drop(&mut self) {
        self.internals.shared.notify_worker_stop(self.internals.id);
    }
}

/// Error running a [`Worker`].
#[derive(Debug)]
pub(crate) enum Error {
    Setup(io::Error),
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
            Error::Setup(err) => write!(f, "error setting up worker thread: {err}"),
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
            Error::Setup(err) | Error::Polling(err) => Some(err),
            Error::ProcessInterrupted => None,
            Error::UserFunction(err) => Some(err),
        }
    }
}

/// Spawn all system actors.
#[allow(clippy::assertions_on_constants, clippy::manual_assert_eq)]
fn spawn_system_actors(mut runtime_ref: RuntimeRef) -> ActorRef<Control> {
    log::trace!(worker_id = runtime_ref.internals.worker_id(); "spawning {SYSTEM_ACTORS} system actors");
    let sys_ref =
        runtime_ref.spawn_local(NoSupervisor, actor_fn(comm_actor), (), ActorOptions::SYSTEM);
    let _ = runtime_ref.spawn_local(NoSupervisor, actor_fn(poll_actor), (), ActorOptions::SYSTEM);
    // Keep this up to date, otherwise we'll exit early.
    assert!(SYSTEM_ACTORS == 2);
    sys_ref
}

/// Control message send to the worker threads by the coordinator, handled by
/// [`comm_actor`].
#[allow(variant_size_differences)] // Can't make `Run` smaller.
pub(crate) enum Control {
    /// Runtime has started, i.e. [`rt::Runtime::start`] was called.
    Started,
    /// Process received a signal.
    Signal(process::Signal),
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
async fn comm_actor(mut ctx: actor::Context<Control, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        let internals = &ctx.runtime_ref().internals;
        let timing = ctx.start_trace();
        log::trace!(worker_id = internals.worker_id(), message:? = msg; "processing coordinator message");
        match msg {
            Control::Started => internals.start(),
            Control::Signal(signal) => internals.relay_signal(signal),
            Control::Run(f) => internals.clone().run_user_function(f),
        }
        ctx.finish_trace(timing, "Processing communication message", &[]);
    }
}

/// System actor that polls the shared ring.
async fn poll_actor(mut ctx: actor::Context<!, ThreadLocal>) {
    let mut pollable = ctx
        .runtime_ref()
        .shared_ring_pollable(ctx.runtime_ref().sq());
    while let Some(res) = next(&mut pollable).await {
        if let Err(err) = res {
            log::warn!("error checking if ring is pollable: {err}");
            // NOTE: going to poll the shared ring below just in case.
        }

        let internals = &ctx.runtime_ref().internals;
        let timing = ctx.start_trace();
        log::trace!(worker_id = internals.worker_id(); "polling shared ring");
        if let Err(err) = internals.try_poll_shared_ring() {
            log::warn!("error polling shared ring: {err}");
        }
        ctx.finish_trace(timing, "Polling shared ring", &[]);
    }
}
