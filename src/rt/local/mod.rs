//! Module with shared runtime internals.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io};

use crossbeam_channel::Receiver;
use log::{debug, trace};
use mio::{Events, Poll, Token};

use crate::actor_ref::ActorRef;
use crate::rt::error::StringError;
use crate::rt::local::scheduler::Scheduler;
use crate::rt::process::ProcessId;
use crate::rt::process::ProcessResult;
use crate::rt::{self, shared, RuntimeRef, Signal, Timers, WakerId};
use crate::trace;

mod scheduler;

/// Number of processes to run in between calls to poll.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Token used to indicate user space events have happened.
pub(super) const WAKER: Token = Token(usize::MAX);
/// Token used to indicate a message was received on the communication channel.
const COMMS: Token = Token(usize::MAX - 1);

/// The runtime that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct Runtime {
    /// Internals of the runtime, shared with zero or more [`RuntimeRef`]s.
    internals: Rc<RuntimeInternals>,
    /// Mio events container.
    events: Events,
    /// Receiving side of the channel for waker events, see the [`rt::waker`]
    /// module for the implementation.
    waker_events: Receiver<ProcessId>,
    /// Two-way communication channel exchange control messages.
    channel: rt::channel::Handle<!, Control>,
    /// Whether or not the runtime was started.
    /// This is here because the worker threads are started before
    /// [`rt::Runtime::start`] is called and thus before any actors are added to
    /// the runtime. Because of this the worker could check all schedulers, see
    /// that no actors are in them and determine it's done before even starting
    /// the runtime.
    ///
    /// [`Runtime::start`]: rt::Runtime::start
    started: bool,
    /// Log used for tracing, `None` is tracing is disabled.
    // TODO: move to `RuntimeInternals`?
    trace_log: Option<trace::Log>,
}

impl Runtime {
    /// Create a new local `Runtime`.
    pub(super) fn new(
        poll: Poll,
        waker_id: WakerId,
        waker_events: Receiver<ProcessId>,
        mut channel: rt::channel::Handle<!, Control>,
        shared_internals: Arc<shared::RuntimeInternals>,
        trace_log: Option<trace::Log>,
        cpu: Option<usize>,
    ) -> io::Result<Runtime> {
        // Register the channel to the coordinator.
        channel.register(poll.registry(), COMMS)?;

        // Finally create all the runtime internals.
        let internals = RuntimeInternals::new(shared_internals, waker_id, poll, cpu);
        Ok(Runtime {
            internals: Rc::new(internals),
            events: Events::with_capacity(128),
            waker_events,
            channel,
            started: false,
            trace_log,
        })
    }

    /// Create a new local `Runtime` for testing.
    ///
    /// Used in the [`crate::test`] module.
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn new_test(shared_internals: Arc<shared::RuntimeInternals>) -> io::Result<Runtime> {
        let poll = Poll::new()?;

        // TODO: this channel will grow unbounded as the waker implementation
        // sends pids into it.
        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(poll.registry(), WAKER)?;
        let waker_id = rt::waker::init(waker, waker_sender);

        let (_, mut channel) = rt::channel::new()?;
        channel.register(poll.registry(), COMMS)?;

        let internals = RuntimeInternals::new(shared_internals, waker_id, poll, None);
        Ok(Runtime {
            internals: Rc::new(internals),
            events: Events::with_capacity(1),
            waker_events,
            channel,
            started: false,
            trace_log: None,
        })
    }

    /// Returns the trace log, if any.
    pub(super) fn trace_log(&mut self) -> &mut Option<trace::Log> {
        &mut self.trace_log
    }

    /// Create a new reference to this runtime.
    pub(crate) fn create_ref(&self) -> RuntimeRef {
        RuntimeRef {
            internals: self.internals.clone(),
        }
    }

    /// Run the runtime's event loop.
    pub(super) fn run_event_loop(mut self) -> Result<(), Error> {
        debug!("running runtime's event loop");
        // Runtime reference used in running the processes.
        let mut runtime_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            trace!("running processes");
            let mut has_local = true;
            for _ in 0..RUN_POLL_RATIO {
                if has_local && self.run_local_process(&mut runtime_ref) {
                    // Only run a single process per iteration.
                    continue;
                } else {
                    // Don't check next iteration.
                    has_local = false;
                }

                if self.run_shared_process(&mut runtime_ref) {
                    // Only run a single process per iteration.
                    continue;
                }

                if !self.internals.scheduler.borrow().has_process()
                    && !self.internals.shared.has_process()
                    // Don't want to exit before the runtime was started.
                    && self.started
                {
                    debug!("no processes to run, stopping runtime");
                    return Ok(());
                }

                // No processes ready to run, move to scheduling them.
                break;
            }

            self.schedule_processes()?;
        }
    }

    /// Attempts to run a single local process. Returns `true` if it ran a
    /// process, `false` otherwise.
    fn run_local_process(&mut self, runtime_ref: &mut RuntimeRef) -> bool {
        let process = self.internals.scheduler.borrow_mut().next_process();
        if let Some(mut process) = process {
            let timing = trace::start(&self.trace_log);
            let pid = process.as_ref().id();
            let name = process.as_ref().name();
            match process.as_mut().run(runtime_ref) {
                ProcessResult::Complete => {}
                ProcessResult::Pending => {
                    self.internals.scheduler.borrow_mut().add_process(process);
                }
            }
            trace::finish(
                &mut self.trace_log,
                timing,
                "Running thread-local process",
                &[("id", &pid.0), ("name", &name)],
            );
            true
        } else {
            false
        }
    }

    /// Attempts to run a single shared process. Returns `true` if it ran a
    /// process, `false` otherwise.
    fn run_shared_process(&mut self, runtime_ref: &mut RuntimeRef) -> bool {
        let process = self.internals.shared.remove_process();
        if let Some(mut process) = process {
            let timing = trace::start(&self.trace_log);
            let pid = process.as_ref().id();
            let name = process.as_ref().name();
            match process.as_mut().run(runtime_ref) {
                ProcessResult::Complete => {
                    self.internals.shared.complete(process);
                }
                ProcessResult::Pending => {
                    self.internals.shared.add_process(process);
                }
            }
            trace::finish(
                &mut self.trace_log,
                timing,
                "Running thread-safe process",
                &[("id", &pid.0), ("name", &name)],
            );
            true
        } else {
            false
        }
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> Result<(), Error> {
        trace!("polling event sources to schedule processes");
        let timing = trace::start(&self.trace_log);
        let mut amount = 0;
        amount += self.schedule_from_os_events()?;
        amount += self.schedule_from_waker();
        amount += self.schedule_from_local_timers();
        amount += self.schedule_from_shared_timers();
        trace::finish(
            &mut self.trace_log,
            timing,
            "Scheduling processes",
            &[("amount", &amount)],
        );
        Ok(())
    }

    /// Schedule processes based on OS events. First polls for events and
    /// schedules processes based on them.
    fn schedule_from_os_events(&mut self) -> Result<usize, Error> {
        // Start with polling for OS events.
        self.poll_os().map_err(Error::Polling)?;

        // Based on the OS event scheduler thread-local processes.
        let timing = trace::start(&self.trace_log);
        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut check_comms = false;
        let mut amount = 0;
        for event in self.events.iter() {
            trace!("Got OS event: {:?}", event);
            match event.token() {
                WAKER => { /* Need to wake up to handle user space events. */ }
                COMMS => check_comms = true,
                token => {
                    scheduler.mark_ready(token.into());
                    amount += 1;
                }
            }
        }
        trace::finish(&mut self.trace_log, timing, "Handling OS events", &[]);

        if check_comms {
            // Don't need this anymore.
            drop(scheduler);
            self.check_comms().map(|()| amount)
        } else {
            Ok(amount)
        }
    }

    /// Schedule processes based on user space waker events, e.g. used by the
    /// `Future` task system.
    fn schedule_from_waker(&mut self) -> usize {
        trace!("polling wakup events");
        let timing = trace::start(&self.trace_log);

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.waker_events.try_iter() {
            scheduler.mark_ready(pid);
            amount += 1;
        }

        trace::finish(
            &mut self.trace_log,
            timing,
            "Scheduling thread-local processes based on wake-up events",
            &[("amount", &amount)],
        );
        amount
    }

    /// Schedule processes based on local timers.
    fn schedule_from_local_timers(&mut self) -> usize {
        trace!("polling local timers");
        let timing = trace::start(&self.trace_log);

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.internals.timers.borrow_mut().deadlines() {
            scheduler.mark_ready(pid);
            amount += 1;
        }

        trace::finish(
            &mut self.trace_log,
            timing,
            "Scheduling thread-local processes based on timers",
            &[("amount", &amount)],
        );
        amount
    }

    /// Schedule processes based on shared timers.
    fn schedule_from_shared_timers(&mut self) -> usize {
        trace!("polling shared timers");
        let timing = trace::start(&self.trace_log);

        let mut amount = 0;
        while let Some(pid) = self.internals.shared.remove_deadline(Instant::now()) {
            self.internals.shared.mark_ready(pid);
            amount += 1;
        }

        trace::finish(
            &mut self.trace_log,
            timing,
            "Scheduling thread-safe processes based on timers",
            &[("amount", &amount)],
        );
        amount
    }

    /// Poll for OS events.
    fn poll_os(&mut self) -> io::Result<()> {
        let timing = trace::start(&self.trace_log);
        let timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is non zero.
        let marked_polling = if timeout.map_or(true, |t| !t.is_zero()) {
            rt::waker::mark_polling(self.internals.waker_id, true);
            true
        } else {
            false
        };

        trace!("polling OS events: timeout={:?}", timeout);
        let res = self
            .internals
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout);

        if marked_polling {
            rt::waker::mark_polling(self.internals.waker_id, false);
        }

        trace::finish(&mut self.trace_log, timing, "Polling for OS events", &[]);
        res
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

        if let Some(deadline) = self.internals.timers.borrow().next_deadline() {
            let now = Instant::now();
            return if deadline <= now {
                // Deadline has already expired, so no blocking.
                Some(Duration::ZERO)
            } else {
                // Check the shared timers with the current deadline.
                let timeout = Some(deadline.duration_since(now));
                self.internals.shared.next_timeout(now, timeout)
            };
        }

        // If there are no local timers check the shared timers.
        self.internals.shared.next_timeout(Instant::now(), None)
    }

    /// Process messages from the communication channel.
    fn check_comms(&mut self) -> Result<(), Error> {
        trace!("processing messages");
        let timing = trace::start(&self.trace_log);
        while let Some(msg) = self.channel.try_recv().map_err(Error::RecvMsg)? {
            match msg {
                Control::Started => self.started = true,
                Control::Signal(signal) => self.relay_signal(signal)?,
                Control::Run(f) => self.run_user_function(f)?,
            }
        }
        trace::finish(
            &mut self.trace_log,
            timing,
            "Processing communication message(s)",
            &[],
        );
        Ok(())
    }

    /// Relay a process `signal` to all actors that wanted to receive it, or
    /// returns an error if no actors want to receive it.
    fn relay_signal(&mut self, signal: Signal) -> Result<(), Error> {
        let timing = trace::start(&self.trace_log);
        trace!("received process signal: {:?}", signal);

        let mut receivers = self.internals.signal_receivers.borrow_mut();
        let res = if signal.should_stop() && receivers.is_empty() {
            Err(Error::ProcessInterrupted)
        } else {
            for receiver in receivers.iter_mut() {
                // TODO: maybe log if we can't send signal?
                let _ = receiver.try_send(signal);
            }
            Ok(())
        };

        trace::finish(
            &mut self.trace_log,
            timing,
            "Relaying process signal to actors",
            &[("signal", &signal.as_str())],
        );
        res
    }

    /// Run user function `f`.
    fn run_user_function(
        &mut self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String>>,
    ) -> Result<(), Error> {
        let timing = trace::start(&self.trace_log);
        trace!("running user function");
        let runtime_ref = self.create_ref();
        let res = f(runtime_ref).map_err(|err| Error::UserFunction(err.into()));
        trace::finish(&mut self.trace_log, timing, "Running user function", &[]);
        res
    }
}

/// Control the [`Runtime`].
#[allow(variant_size_differences)] // Can't make `Run` smaller.
pub(crate) enum Control {
    /// Runtime has started, i.e. [`rt::Runtime::start`] was called.
    Started,
    /// Process received a signal.
    Signal(Signal),
    /// Run a user defined function.
    Run(Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>),
}

/// Error running a [`Runtime`].
#[derive(Debug)]
pub(crate) enum Error {
    /// Error in [`Runtime::new`].
    Init(io::Error),
    /// Error polling [`Poll`].
    Polling(io::Error),
    /// Error receiving message from communication channel.
    RecvMsg(io::Error),
    /// Process was interrupted (i.e. received process signal), but no actor can
    /// receive the signal.
    ProcessInterrupted,
    /// Error running user function.
    UserFunction(StringError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            Init(err) => write!(f, "error initialising local runtime: {}", err),
            Polling(err) => write!(f, "error polling OS: {}", err),
            RecvMsg(err) => write!(f, "error receiving message(s): {}", err),
            ProcessInterrupted => write!(
                f,
                "received process signal, but no receivers for it: stopping runtime"
            ),
            UserFunction(err) => write!(f, "error running user function: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            Init(ref err) | Polling(ref err) | RecvMsg(ref err) => Some(err),
            ProcessInterrupted => None,
            UserFunction(ref err) => Some(err),
        }
    }
}

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
pub(super) struct RuntimeInternals {
    /// Runtime internals shared between coordinator and worker threads.
    pub(super) shared: Arc<shared::RuntimeInternals>,
    /// Waker id used to create a `Waker` for thread-local actors.
    pub(super) waker_id: WakerId,
    /// Scheduler for thread-local actors.
    pub(super) scheduler: RefCell<Scheduler>,
    /// OS poll, used for event notifications to support non-blocking I/O.
    pub(super) poll: RefCell<Poll>,
    /// Timers, deadlines and timeouts.
    pub(crate) timers: RefCell<Timers>,
    /// Actor references to relay received `Signal`s to.
    pub(super) signal_receivers: RefCell<Vec<ActorRef<Signal>>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    pub(super) cpu: Option<usize>,
}

impl RuntimeInternals {
    /// Create a local runtime internals.
    pub(super) fn new(
        shared_internals: Arc<shared::RuntimeInternals>,
        waker_id: WakerId,
        poll: Poll,
        cpu: Option<usize>,
    ) -> RuntimeInternals {
        RuntimeInternals {
            shared: shared_internals,
            waker_id,
            scheduler: RefCell::new(Scheduler::new()),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(Vec::new()),
            cpu,
        }
    }
}
