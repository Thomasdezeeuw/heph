//! Module with shared runtime internals.

use std::cell::{RefCell, RefMut};
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io};

use crossbeam_channel::Receiver;
use log::{debug, info, trace};
use mio::{Events, Poll, Token};

use crate::actor_ref::{ActorGroup, Delivery, SendError};
use crate::rt::error::StringError;
use crate::rt::process::ProcessId;
use crate::rt::process::ProcessResult;
use crate::rt::{self, cpu_usage, shared, RuntimeRef, Signal, WakerId};
use crate::trace;

mod scheduler;
mod timers;

use scheduler::Scheduler;
use timers::Timers;

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
/// Token used to indicate the shared [`Poll`] (in [`shared::RuntimeInternals`])
/// has events.
const SHARED_POLL: Token = Token(usize::MAX - 2);

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
    /// Communication channel exchange control messages.
    channel: rt::channel::Receiver<Control>,
    /// Whether or not the runtime was started.
    /// This is here because the worker threads are started before
    /// [`rt::Runtime::start`] is called and thus before any actors are added to
    /// the runtime. Because of this the worker could check all schedulers, see
    /// that no actors are in them and determine it's done before even starting
    /// the runtime.
    ///
    /// [`Runtime::start`]: rt::Runtime::start
    started: bool,
}

/// Run a block of code, catching panics when testing or not otherwise.
macro_rules! catch_in_test {
    ($fmt: tt, $code: block) => {
        #[cfg(any(test, feature = "test"))]
        if let Err(err) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $code)) {
            let msg = match err.downcast_ref::<&'static str>() {
                Some(s) => *s,
                None => match err.downcast_ref::<String>() {
                    Some(s) => &**s,
                    None => "<unknown>",
                },
            };
            eprintln!($fmt, msg);
        }
        #[cfg(not(any(test, feature = "test")))]
        $code
    };
}

impl Runtime {
    /// Create a new local `Runtime`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: NonZeroUsize,
        poll: Poll,
        waker_id: WakerId,
        waker_events: Receiver<ProcessId>,
        mut channel: rt::channel::Receiver<Control>,
        shared_internals: Arc<shared::RuntimeInternals>,
        trace_log: Option<trace::Log>,
        cpu: Option<usize>,
    ) -> io::Result<Runtime> {
        // Register the shared poll intance.
        shared_internals.register_worker_poll(poll.registry(), SHARED_POLL)?;
        // Register the channel to the coordinator.
        channel.register(poll.registry(), COMMS)?;

        // Finally create all the runtime internals.
        let internals = RuntimeInternals::new(id, shared_internals, waker_id, poll, cpu, trace_log);
        Ok(Runtime {
            internals: Rc::new(internals),
            events: Events::with_capacity(128),
            waker_events,
            channel,
            started: false,
        })
    }

    /// Create a new local `Runtime` for testing.
    ///
    /// Used in the [`crate::test`] module.
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn new_test(
        shared_internals: Arc<shared::RuntimeInternals>,
        mut channel: rt::channel::Receiver<Control>,
    ) -> io::Result<Runtime> {
        let poll = Poll::new()?;

        // TODO: this channel will grow unbounded as the waker implementation
        // sends pids into it.
        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(poll.registry(), WAKER)?;
        let waker_id = rt::waker::init(waker, waker_sender);

        channel.register(poll.registry(), COMMS)?;

        let id = NonZeroUsize::new(usize::MAX).unwrap();
        let internals = RuntimeInternals::new(id, shared_internals, waker_id, poll, None, None);
        Ok(Runtime {
            internals: Rc::new(internals),
            events: Events::with_capacity(1),
            waker_events,
            channel,
            started: false,
        })
    }

    /// Returns the trace log, if any.
    pub(super) fn trace_log(&mut self) -> RefMut<'_, Option<trace::Log>> {
        self.internals.trace_log.borrow_mut()
    }

    /// Create a new reference to this runtime.
    pub(crate) fn create_ref(&self) -> RuntimeRef {
        RuntimeRef {
            internals: self.internals.clone(),
        }
    }

    /// Run the runtime's event loop.
    pub(crate) fn run_event_loop(mut self) -> Result<(), Error> {
        debug!("running runtime's event loop");
        // Runtime reference used in running the processes.
        let mut runtime_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            trace!("running processes");
            let mut n = 0;
            while n < RUN_POLL_RATIO {
                if !self.run_local_process(&mut runtime_ref) {
                    break;
                }
                n += 1;
            }
            while n < RUN_POLL_RATIO {
                if !self.run_shared_process(&mut runtime_ref) {
                    break;
                }
                n += 1;
            }

            if self.started && !self.has_process() {
                debug!("no processes to run, stopping runtime");
                self.internals.shared.wake_all_workers();
                return Ok(());
            }

            self.schedule_processes()?;
        }
    }

    /// Attempts to run a single local process. Returns `true` if it ran a
    /// process, `false` otherwise.
    fn run_local_process(&mut self, runtime_ref: &mut RuntimeRef) -> bool {
        let process = self.internals.scheduler.borrow_mut().next_process();
        match process {
            Some(mut process) => {
                catch_in_test!("Thread-local actor panicked: '{}'", {
                    let timing = trace::start(&*self.internals.trace_log.borrow());
                    let pid = process.as_ref().id();
                    let name = process.as_ref().name();
                    match process.as_mut().run(runtime_ref) {
                        ProcessResult::Complete => {}
                        ProcessResult::Pending => {
                            self.internals.scheduler.borrow_mut().add_process(process);
                        }
                    }
                    trace::finish_rt(
                        self.internals.trace_log.borrow_mut().as_mut(),
                        timing,
                        "Running thread-local process",
                        &[("id", &pid.0), ("name", &name)],
                    );
                });
                true
            }
            None => false,
        }
    }

    /// Attempts to run a single shared process. Returns `true` if it ran a
    /// process, `false` otherwise.
    fn run_shared_process(&mut self, runtime_ref: &mut RuntimeRef) -> bool {
        let process = self.internals.shared.remove_process();
        match process {
            Some(mut process) => {
                catch_in_test!("Thread-safe actor panicked: '{}'", {
                    let timing = trace::start(&*self.internals.trace_log.borrow());
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
                    trace::finish_rt(
                        self.internals.trace_log.borrow_mut().as_mut(),
                        timing,
                        "Running thread-safe process",
                        &[("id", &pid.0), ("name", &name)],
                    );
                });
                true
            }
            None => false,
        }
    }

    /// Returns `true` if there are processes in either the local or shared
    /// schedulers.
    fn has_process(&self) -> bool {
        self.internals.scheduler.borrow().has_process() || self.internals.shared.has_process()
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> Result<(), Error> {
        trace!("polling event sources to schedule processes");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        // Schedule local and shared processes based on various event sources.
        let (mut local_amount, check_shared_poll) = self.schedule_from_os_events()?;
        let mut shared_amount = if check_shared_poll {
            self.schedule_from_shared_os_events()
                .map_err(Error::Polling)?
        } else {
            0
        };
        local_amount += self.schedule_from_waker();
        let now = Instant::now();
        local_amount += self.schedule_from_local_timers(now);
        shared_amount += self.schedule_from_shared_timers(now);

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

    /// Schedule processes based on OS events. First polls for events and
    /// schedules processes based on them.
    ///
    /// Returns the amount of processes marked as active and a boolean
    /// indicating whether or not the shared timers should be checked.
    fn schedule_from_os_events(&mut self) -> Result<(usize, bool), Error> {
        // Start with polling for OS events.
        self.poll_os().map_err(Error::Polling)?;

        // Based on the OS event scheduler thread-local processes.
        let timing = trace::start(&*self.internals.trace_log.borrow());
        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut check_comms = false;
        let mut check_shared_poll = false;
        let mut amount = 0;
        for event in self.events.iter() {
            trace!("got OS event: {:?}", event);
            match event.token() {
                WAKER => { /* Need to wake up to handle user space events. */ }
                COMMS => check_comms = true,
                SHARED_POLL => check_shared_poll = true,
                token => {
                    let pid = token.into();
                    trace!(
                        "scheduling local process based on OS event: pid={}, event={:?}",
                        pid,
                        event
                    );
                    scheduler.mark_ready(pid);
                    amount += 1;
                }
            }
        }
        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Handling OS events",
            &[],
        );

        if check_comms {
            // Don't need this anymore.
            drop(scheduler);
            self.check_comms()?;
        }
        Ok((amount, check_shared_poll))
    }

    /// Schedule processes based on shared OS events.
    fn schedule_from_shared_os_events(&mut self) -> io::Result<usize> {
        trace!("polling shared OS events");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut amount = 0;
        if self.internals.shared.try_poll(&mut self.events)? {
            for event in self.events.iter() {
                trace!("got shared OS event: {:?}", event);
                let pid = event.token().into();
                trace!(
                    "scheduling shared process based on OS event: pid={}, event={:?}",
                    pid,
                    event
                );
                self.internals.shared.mark_ready(pid);
                amount += 1;
            }
        }

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Scheduling thread-safe processes based on shared OS events",
            &[("amount", &amount)],
        );
        Ok(amount)
    }

    /// Schedule processes based on user space waker events, e.g. used by the
    /// `Future` task system.
    fn schedule_from_waker(&mut self) -> usize {
        trace!("polling wakup events");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.waker_events.try_iter() {
            trace!("waking up local process: pid={}", pid);
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
        trace!("polling local timers");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.internals.timers.borrow_mut().deadlines(now) {
            trace!("expiring timer for local process: pid={}", pid);
            scheduler.mark_ready(pid);
            amount += 1;
        }

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
        trace!("polling shared timers");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut amount: usize = 0;
        while let Some(pid) = self.internals.shared.remove_next_deadline(now) {
            trace!("expiring timer for shared process: pid={}", pid);
            self.internals.shared.mark_ready(pid);
            amount += 1;
        }

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
            trace!("waking {} worker threads", wake_n);
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
        let mut timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is non zero.
        let marked_polling = if timeout.map_or(true, |t| !t.is_zero()) {
            rt::waker::mark_polling(self.internals.waker_id, true);
            // We need to check the timeout here to ensure we didn't miss any
            // wake-ups/timers since we determined the timeout and marked
            // ourselves as polling above.
            //
            // It could be that between the two calls (`determine_timeout` and
            // `mark_polling`) we received e.g. a wake-up event but didn't
            // consider it in `determine_timeout`. But because we didn't mark
            // ourselves as polling yet we also won't be awoken from polling,
            // causing us to poll for ever, missing the wake-up. That would look
            // something like the following:
            //
            // Thread 0                    | Thread 1
            // 1. determine_timeout = None |
            //                             | 2. task::Waker: wake-up pid=1.
            //                             | 3. ThreadWaker: not polling -> no wakeup.
            // 4. mark_polling(true)       |
            // 5. poll(None)               |
            // 6. Waiting for ever...      |
            timeout = self.check_timeout(timeout);
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

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Polling for OS events",
            &[],
        );
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

    /// Check if `timeout` is still correct in regard to shared event resources,
    /// i.e. wake-ups, shared scheduler and timers.
    fn check_timeout(&self, timeout: Option<Duration>) -> Option<Duration> {
        // NOTE: we don't have to check local resources as those can't be
        // changed from outside this thread.
        if !self.waker_events.is_empty() || self.internals.shared.has_ready_process() {
            Some(Duration::ZERO)
        } else {
            self.internals.shared.next_timeout(Instant::now(), timeout)
        }
    }

    /// Process messages from the communication channel.
    fn check_comms(&mut self) -> Result<(), Error> {
        trace!("processing messages");
        let timing = trace::start(&*self.internals.trace_log.borrow());
        while let Some(msg) = self.channel.try_recv().map_err(Error::RecvMsg)? {
            match msg {
                Control::Started => self.started = true,
                Control::Signal(signal) => {
                    if let Signal::User2 = signal {
                        let timing = trace::start(&*self.internals.trace_log.borrow());
                        let metrics = self.internals.metrics();
                        info!(target: "metrics", "metrics: {:?}", metrics);
                        trace::finish_rt(
                            self.internals.trace_log.borrow_mut().as_mut(),
                            timing,
                            "Printing runtime metrics",
                            &[],
                        );
                    }

                    self.relay_signal(signal)?
                }
                Control::Run(f) => self.run_user_function(f)?,
            }
        }
        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Processing communication message(s)",
            &[],
        );
        Ok(())
    }

    /// Relay a process `signal` to all actors that wanted to receive it, or
    /// returns an error if no actors want to receive it.
    fn relay_signal(&mut self, signal: Signal) -> Result<(), Error> {
        let timing = trace::start(&*self.internals.trace_log.borrow());
        trace!("received process signal: {:?}", signal);

        let mut receivers = self.internals.signal_receivers.borrow_mut();
        receivers.remove_disconnected();
        let res = match receivers.try_send(signal, Delivery::ToAll) {
            Ok(()) => Ok(()),
            Err(SendError) if signal.should_stop() => Err(Error::ProcessInterrupted),
            Err(SendError) => Ok(()),
        };

        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
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
        let timing = trace::start(&*self.internals.trace_log.borrow());
        trace!("running user function");
        let runtime_ref = self.create_ref();
        let res = f(runtime_ref).map_err(|err| Error::UserFunction(err.into()));
        trace::finish_rt(
            self.internals.trace_log.borrow_mut().as_mut(),
            timing,
            "Running user function",
            &[],
        );
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

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Control::*;
        f.write_str("Control::")?;
        match self {
            Started => f.write_str("Started"),
            Signal(signal) => f.debug_tuple("Signal").field(&signal).finish(),
            Run(..) => f.write_str("Run(..)"),
        }
    }
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
    /// Unique id among the worker threads.
    id: NonZeroUsize,
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
    pub(super) signal_receivers: RefCell<ActorGroup<Signal>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    pub(super) cpu: Option<usize>,
    /// Log used for tracing, `None` is tracing is disabled.
    pub(super) trace_log: RefCell<Option<trace::Log>>,
}

/// Metrics for [`RuntimeInternals`].
#[derive(Debug)]
#[allow(dead_code)] // https://github.com/rust-lang/rust/issues/88900.
pub(crate) struct Metrics {
    id: NonZeroUsize,
    scheduler: scheduler::Metrics,
    timers: timers::Metrics,
    process_signal_receivers: usize,
    cpu_affinity: Option<usize>,
    cpu_time: Duration,
    trace_log: Option<trace::Metrics>,
}

impl RuntimeInternals {
    /// Create a local runtime internals.
    pub(super) fn new(
        id: NonZeroUsize,
        shared_internals: Arc<shared::RuntimeInternals>,
        waker_id: WakerId,
        poll: Poll,
        cpu: Option<usize>,
        trace_log: Option<trace::Log>,
    ) -> RuntimeInternals {
        RuntimeInternals {
            id,
            shared: shared_internals,
            waker_id,
            scheduler: RefCell::new(Scheduler::new()),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(ActorGroup::empty()),
            cpu,
            trace_log: RefCell::new(trace_log),
        }
    }

    /// Gather metrics about the runtime internals.
    fn metrics(&self) -> Metrics {
        let cpu_time = cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID);
        Metrics {
            id: self.id,
            scheduler: self.scheduler.borrow().metrics(),
            timers: self.timers.borrow_mut().metrics(),
            process_signal_receivers: self.signal_receivers.borrow().len(),
            cpu_affinity: self.cpu,
            trace_log: self.trace_log.borrow().as_ref().map(trace::Log::metrics),
            cpu_time,
        }
    }
}
