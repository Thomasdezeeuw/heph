//! Worker thread code.

use std::cell::RefCell;
#[cfg(target_os = "linux")]
use std::mem;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io, thread};

use crossbeam_channel::{self, Receiver};
#[cfg(target_os = "linux")]
use log::warn;
use log::{debug, error, trace};
use mio::{Events, Poll, Registry, Token};

use crate::rt::hack::SetupFn;
use crate::rt::process::ProcessResult;
use crate::rt::scheduler::LocalScheduler;
use crate::rt::timers::Timers;
use crate::rt::waker::ThreadWaker;
use crate::rt::{self, shared, ProcessId, RuntimeInternal, RuntimeRef, Signal};
use crate::trace;

/// Handle to a worker thread.
pub(super) struct Worker<E> {
    /// Unique id (among all threads in the `Runtime`).
    id: NonZeroUsize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), rt::Error<E>>>,
    /// Two-way communication channel to share messages with the worker thread.
    channel: rt::channel::Handle<CoordinatorMessage, WorkerMessage>,
    /// Initialy this will be `None`, but once the worker thread is setup this
    /// will be set to `Some` and can be used as an optimisation over sending
    /// `CoordinatorMessage::Waker` messages.
    thread_waker: Option<&'static ThreadWaker>,
}

/// Message send by the coordinator thread.
#[derive(Debug)]
pub(crate) enum CoordinatorMessage {
    /// Process received a signal.
    Signal(Signal),
    /// Signal to wake-up the worker thread.
    Wake,
}

/// Message send by the worker thread.
#[derive(Debug)]
pub(crate) enum WorkerMessage {
    /// Worker thread is setup and send `ThreadWaker` as an optimised way to
    /// wake the thread.
    Waker(&'static ThreadWaker),
}

impl<E> Worker<E> {
    /// Start a new worker thread.
    pub(super) fn start<S>(
        id: NonZeroUsize,
        setup: Option<S>,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> io::Result<Worker<S::Error>>
    where
        S: SetupFn<Error = E>,
        E: Send + 'static,
    {
        rt::channel::new().and_then(|(channel, worker_handle)| {
            thread::Builder::new()
                .name(format!("Worker {}", id))
                .spawn(move || {
                    main(
                        id,
                        setup,
                        worker_handle,
                        shared_internals,
                        auto_cpu_affinity,
                        trace_log,
                    )
                })
                .map(|handle| Worker {
                    id,
                    handle,
                    channel,
                    thread_waker: None,
                })
        })
    }

    /// Return the worker's id.
    pub(super) fn id(&self) -> usize {
        self.id.get()
    }

    /// Checks if the `Worker` is alive.
    pub(super) fn is_alive(&mut self) -> bool {
        self.channel.is_alive()
    }

    /// Registers the channel used to communicate with the thread. Uses the
    /// [`id`] as [`Token`].
    ///
    /// [`id`]: Worker::id
    pub(super) fn register(&mut self, registry: &Registry) -> io::Result<()> {
        self.channel.register(registry, Token(self.id()))
    }

    /// Send the worker thread a `signal`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        let msg = CoordinatorMessage::Signal(signal);
        self.channel.try_send(msg)
    }

    /// Wake the worker thread. Returns `true` if the thread is awoken, `false`
    /// otherwise.
    pub(super) fn wake(&mut self) -> io::Result<bool> {
        if let Some(thread_waker) = self.thread_waker {
            thread_waker.wake_thread()
        } else {
            let msg = CoordinatorMessage::Wake;
            self.channel.try_send(msg).map(|()| true)
        }
    }

    /// Handle all incoming messages.
    pub(super) fn handle_messages(&mut self) -> io::Result<()> {
        while let Some(msg) = self.channel.try_recv()? {
            match msg {
                WorkerMessage::Waker(thread_waker) => self.thread_waker = Some(thread_waker),
            }
        }
        Ok(())
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<Result<(), rt::Error<E>>> {
        self.handle.join()
    }
}

/// Run a worker thread, with an optional `setup` function.
fn main<S>(
    id: NonZeroUsize,
    setup: Option<S>,
    receiver: rt::channel::Handle<WorkerMessage, CoordinatorMessage>,
    shared_internals: Arc<shared::RuntimeInternals>,
    auto_cpu_affinity: bool,
    mut trace_log: Option<trace::Log>,
) -> Result<(), rt::Error<S::Error>>
where
    S: SetupFn,
{
    let timing = trace::start(&trace_log);

    #[cfg(target_os = "linux")]
    let cpu = if auto_cpu_affinity {
        let cpu = id.get() - 1; // Worker ids start at 1, cpus at 0.
        let cpu_set = cpu_set(cpu);
        match set_affinity(&cpu_set) {
            Ok(()) => {
                debug!("worker thread using CPU '{}'", cpu);
                Some(cpu)
            }
            Err(err) => {
                warn!("error setting CPU affinity: {}", err);
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let cpu = {
        let _ = (id, auto_cpu_affinity); // Silence unused variables warnings.
        None
    };

    let runtime = RunningRuntime::init(receiver, shared_internals, cpu)
        .map_err(|err| rt::Error::worker(Error::Init(err)))?;

    trace::finish(
        &mut trace_log,
        timing,
        "Initialising the worker thread",
        &[],
    );

    // Run optional setup.
    if let Some(setup) = setup {
        let timing = trace::start(&trace_log);
        let runtime_ref = runtime.create_ref();
        setup.setup(runtime_ref).map_err(rt::Error::setup)?;
        trace::finish(&mut trace_log, timing, "Running user setup function", &[]);
    }

    // All setup is done, so we're ready to run the event loop.
    runtime.run_event_loop(&mut trace_log)
}

/// Create a cpu set that may only run on `cpu`.
#[cfg(target_os = "linux")]
fn cpu_set(cpu: usize) -> libc::cpu_set_t {
    let mut cpu_set = unsafe { mem::zeroed() };
    unsafe { libc::CPU_ZERO(&mut cpu_set) };
    unsafe { libc::CPU_SET(cpu % libc::CPU_SETSIZE as usize, &mut cpu_set) };
    cpu_set
}

/// Set the affinity of this thread to the `cpu_set`.
#[cfg(target_os = "linux")]
fn set_affinity(cpu_set: &libc::cpu_set_t) -> io::Result<()> {
    let thread = unsafe { libc::pthread_self() };
    let res = unsafe { libc::pthread_setaffinity_np(thread, mem::size_of_val(cpu_set), cpu_set) };
    if res == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Error running a [`Worker`].
#[derive(Debug)]
pub(super) enum Error {
    /// Error in [`RunningRuntime::init`].
    Init(io::Error),
    /// Error polling [`mio::Poll`].
    Polling(io::Error),
    /// Error receiving message on coordinator channel.
    RecvMsg(io::Error),
    /// Process was interrupted (i.e. received process signal), but no actor can
    /// receive the signal.
    ProcessInterrupted,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            Init(err) => write!(f, "error initialising worker: {}", err),
            Polling(err) => write!(f, "error polling for events: {}", err),
            RecvMsg(err) => write!(f, "error receiving message from coordinator: {}", err),
            ProcessInterrupted => write!(
                f,
                "received process signal, but no receivers for it: stopped running"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            Init(ref err) | Polling(ref err) | RecvMsg(ref err) => Some(err),
            ProcessInterrupted => None,
        }
    }
}

/// The runtime that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct RunningRuntime {
    /// Inside of the runtime, shared with zero or more `RuntimeRef`s.
    internal: Rc<RuntimeInternal>,
    /// Mio events container.
    events: Events,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
    /// Two-way communication channel to share messages with the coordinator.
    channel: rt::channel::Handle<WorkerMessage, CoordinatorMessage>,
}

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Id used for the awakener.
const WAKER: Token = Token(usize::max_value());
const COORDINATOR: Token = Token(usize::max_value() - 1);

impl RunningRuntime {
    /// Create a new running runtime.
    pub(crate) fn init(
        mut channel: rt::channel::Handle<WorkerMessage, CoordinatorMessage>,
        shared_internals: Arc<shared::RuntimeInternals>,
        cpu: Option<usize>,
    ) -> io::Result<RunningRuntime> {
        // OS poll for OS event notifications (e.g. TCP connection readable).
        let poll = Poll::new()?;
        // Register the channel to the coordinator.
        channel.register(poll.registry(), COORDINATOR)?;

        // Setup the waking mechanism.
        // First it needs a user-space queue.
        let (waker_sender, waker_recv) = crossbeam_channel::unbounded();
        // Next a way to wake us from polling the OS.
        let waker = mio::Waker::new(poll.registry(), WAKER)?;
        // With both we create a `rt::waker::ThreadWaker`.
        let waker_id = rt::waker::init(waker, waker_sender);
        let thread_waker = rt::waker::get_thread_waker(waker_id);
        // And send it to the coordinator so it can optimise the waking process
        // (of waking this worker thread).
        channel.try_send(WorkerMessage::Waker(thread_waker))?;

        // Finally create all the runtime internals.
        let internal = RuntimeInternal {
            shared: shared_internals,
            waker_id,
            scheduler: RefCell::new(LocalScheduler::new()),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(Vec::new()),
            cpu,
        };
        Ok(RunningRuntime {
            internal: Rc::new(internal),
            events: Events::with_capacity(128),
            waker_events: waker_recv,
            channel,
        })
    }

    /// Create a new reference to this runtime.
    pub(crate) fn create_ref(&self) -> RuntimeRef {
        RuntimeRef {
            internal: self.internal.clone(),
        }
    }

    /// Run the runtime's event loop.
    fn run_event_loop<E>(mut self, trace_log: &mut Option<trace::Log>) -> Result<(), rt::Error<E>> {
        debug!("running runtime's event loop");
        // Runtime reference used in running the processes.
        let mut runtime_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            trace!("running processes");
            for _ in 0..RUN_POLL_RATIO {
                // NOTE: preferably this running of a process is handled
                // completely within a method of the schedulers, however this is
                // not possible.
                // Because we need `borrow_mut` the scheduler here we couldn't
                // also get a mutable reference to it to add actors, while a
                // process is running. Thus we need to remove a process from the
                // scheduler, drop the mutable reference, and only then run the
                // process. This allow a `RuntimeRef` to also mutable borrow the
                // `Scheduler` to add new actors to it.

                let process = self.internal.scheduler.borrow_mut().next_process();
                if let Some(mut process) = process {
                    let timing = trace::start(&trace_log);
                    let pid = process.as_ref().id();
                    let name = process.as_ref().name();
                    match process.as_mut().run(&mut runtime_ref) {
                        ProcessResult::Complete => {}
                        ProcessResult::Pending => {
                            self.internal.scheduler.borrow_mut().add_process(process);
                        }
                    }
                    trace::finish(
                        trace_log,
                        timing,
                        "Running thread-local process",
                        &[("id", &pid.0), ("name", &name)],
                    );
                    // Only run a single process per iteration.
                    continue;
                }

                let process = self.internal.shared.remove_process();
                if let Some(mut process) = process {
                    let timing = trace::start(&trace_log);
                    let pid = process.as_ref().id();
                    let name = process.as_ref().name();
                    match process.as_mut().run(&mut runtime_ref) {
                        ProcessResult::Complete => {
                            self.internal.shared.complete(process);
                        }
                        ProcessResult::Pending => {
                            self.internal.shared.add_process(process);
                        }
                    }
                    trace::finish(
                        trace_log,
                        timing,
                        "Running thread-safe process",
                        &[("id", &pid.0), ("name", &name)],
                    );
                    // Only run a single process per iteration.
                    continue;
                }

                if !self.internal.scheduler.borrow().has_process()
                    && !self.internal.shared.has_process()
                {
                    debug!("no processes to run, stopping runtime");
                    return Ok(());
                } else {
                    // No processes ready to run.
                    break;
                }
            }

            self.schedule_processes(trace_log)
                .map_err(rt::Error::worker)?;
        }
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self, trace_log: &mut Option<trace::Log>) -> Result<(), Error> {
        trace!("polling event sources to schedule processes");

        // Start with polling for OS events.
        let timing = trace::start(&trace_log);
        self.poll().map_err(Error::Polling)?;
        trace::finish(trace_log, timing, "Polling for OS events", &[]);

        // Based on the OS event scheduler thread-local processes.
        let timing = trace::start(&trace_log);
        let mut scheduler = self.internal.scheduler.borrow_mut();
        let mut check_coordinator = false;
        for event in self.events.iter() {
            trace!("OS event: {:?}", event);
            match event.token() {
                WAKER => {}
                COORDINATOR => check_coordinator = true,
                token => scheduler.mark_ready(token.into()),
            }
        }
        trace::finish(trace_log, timing, "Handling OS events", &[]);

        // User space wake up events, e.g. used by the `Future` task system.
        trace!("polling wakup events");
        let timing = trace::start(&trace_log);
        for pid in self.waker_events.try_iter() {
            scheduler.mark_ready(pid);
        }
        trace::finish(
            trace_log,
            timing,
            "Scheduling thread-local processes based on wake-up events",
            &[],
        );

        // User space timers, powers the `timer` module.
        trace!("polling timers");
        let timing = trace::start(&trace_log);
        for pid in self.internal.timers.borrow_mut().deadlines() {
            scheduler.mark_ready(pid);
        }
        trace::finish(
            trace_log,
            timing,
            "Scheduling thread-local processes based on timers",
            &[],
        );

        if check_coordinator {
            // Don't need this anymore.
            drop(scheduler);
            // Process coordinator messages.
            let timing = trace::start(&trace_log);
            self.check_coordinator()?;
            trace::finish(trace_log, timing, "Process coordinator messages", &[]);
        }
        Ok(())
    }

    /// Poll for OS events.
    fn poll(&mut self) -> io::Result<()> {
        let timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is non zero.
        let mark_waker = if !is_zero(timeout) {
            rt::waker::mark_polling(self.internal.waker_id, true);
            true
        } else {
            false
        };

        trace!("polling OS: timeout={:?}", timeout);
        let res = self
            .internal
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout);

        if mark_waker {
            rt::waker::mark_polling(self.internal.waker_id, false);
        }

        res
    }

    /// Determine the timeout to be used in polling.
    fn determine_timeout(&self) -> Option<Duration> {
        // If there are any processes ready to run, any waker events or user
        // space events we don't want to block.
        if self.internal.scheduler.borrow().has_ready_process()
            || !self.waker_events.is_empty()
            || self.internal.shared.has_ready_process()
        {
            Some(Duration::ZERO)
        } else if let Some(deadline) = self.internal.timers.borrow().next_deadline() {
            let now = Instant::now();
            if deadline <= now {
                // Deadline has already expired, so no blocking.
                Some(Duration::ZERO)
            } else {
                // Time between the deadline and right now.
                Some(deadline.duration_since(now))
            }
        } else {
            // We don't have any reason to return early from polling with an OS
            // event.
            None
        }
    }

    /// Process messages from the coordinator.
    fn check_coordinator(&mut self) -> Result<(), Error> {
        use CoordinatorMessage::*;
        while let Some(msg) = self.channel.try_recv().map_err(Error::RecvMsg)? {
            match msg {
                // Relay a process signal to all actors that wanted to receive
                // it.
                Signal(signal) => {
                    trace!("received process signal: {:?}", signal);
                    let mut receivers = self.internal.signal_receivers.borrow_mut();

                    if receivers.is_empty() && signal.should_stop() {
                        error!(
                            "received {:#} process signal, but there are no receivers for it, stopping runtime",
                            signal
                        );
                        return Err(Error::ProcessInterrupted);
                    }

                    for receiver in receivers.iter_mut() {
                        // Don't care if we succeed in sending the message.
                        let _ = receiver.try_send(signal);
                    }
                }
                Wake => { /* Just need to wake up. */ }
            }
        }
        Ok(())
    }
}

/// Returns `true` is timeout is `Some(Duration::from_nanos(0))`.
fn is_zero(timeout: Option<Duration>) -> bool {
    timeout.map(|t| t.is_zero()).unwrap_or(false)
}
