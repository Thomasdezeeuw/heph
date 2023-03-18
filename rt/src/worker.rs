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
use std::os::fd::{AsFd, AsRawFd};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io, thread};

use crossbeam_channel::{self, Receiver};
use heph::actor_ref::{Delivery, SendError};
use log::{as_debug, debug, info, trace};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Registry, Token};

use crate::error::StringError;
use crate::local::waker::{self, WakerId};
use crate::local::RuntimeInternals;
use crate::process::{ProcessId, ProcessResult};
use crate::setup::set_cpu_affinity;
use crate::thread_waker::ThreadWaker;
use crate::{self as rt, cpu_usage, shared, trace, RuntimeRef, Signal};

/// Number of processes to run in between calls to poll.
///
/// This number is chosen arbitrarily.
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
/// Token used to indicate the I/O uring has events.
const RING: Token = Token(usize::MAX - 3);

/// Setup a new worker thread.
///
/// Use [`WorkerSetup::start`] to spawn the worker thread.
pub(super) fn setup(id: NonZeroUsize) -> io::Result<(WorkerSetup, &'static ThreadWaker)> {
    let poll = Poll::new()?;
    // TODO: configure ring.
    let ring = a10::Ring::new(512)?;

    // Setup the waking mechanism.
    let (waker_sender, waker_events) = crossbeam_channel::unbounded();
    let waker = mio::Waker::new(poll.registry(), WAKER)?;
    let waker_id = waker::init(waker, waker_sender);
    let thread_waker = waker::get_thread_waker(waker_id);

    let setup = WorkerSetup {
        id,
        poll,
        ring,
        waker_id,
        waker_events,
    };
    Ok((setup, thread_waker))
}

/// Setup work required before starting a worker thread, see [`setup`].
pub(super) struct WorkerSetup {
    /// See [`Worker::id`].
    id: NonZeroUsize,
    /// Poll instance for the worker thread. This is needed before starting the
    /// thread to initialise the [`rt::local::waker`].
    poll: Poll,
    /// I/O uring.
    ring: a10::Ring,
    /// Waker id used to create a `Waker` for thread-local actors.
    waker_id: WakerId,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
}

impl WorkerSetup {
    /// Start a new worker thread.
    pub(super) fn start(
        self,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> io::Result<Handle> {
        rt::channel::new().and_then(move |(sender, receiver)| {
            let id = self.id;
            thread::Builder::new()
                .name(format!("Worker {id}"))
                .spawn(move || {
                    let worker = Worker::setup(
                        self,
                        receiver,
                        shared_internals,
                        auto_cpu_affinity,
                        trace_log,
                    )
                    .map_err(rt::Error::worker)?;
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
    pub(super) const fn id(&self) -> usize {
        self.id.get()
    }
}

/// Handle to a worker thread.
#[derive(Debug)]
pub(super) struct Handle {
    /// Unique id (among all threads in the [`rt::Runtime`]).
    id: NonZeroUsize,
    /// Two-way communication channel to share messages with the worker thread.
    channel: rt::channel::Sender<Control>,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), rt::Error>>,
}

impl Handle {
    /// Return the worker's id.
    pub(super) const fn id(&self) -> usize {
        self.id.get()
    }

    /// Registers the channel used to communicate with the thread. Uses the
    /// [`Worker::id`] as [`Token`].
    pub(super) fn register(&mut self, registry: &Registry) -> io::Result<()> {
        self.channel.register(registry, Token(self.id()))
    }

    /// Send the worker thread a signal that the runtime has started.
    pub(super) fn send_runtime_started(&mut self) -> io::Result<()> {
        self.channel.try_send(Control::Started)
    }

    /// Send the worker thread a `signal`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        self.channel.try_send(Control::Signal(signal))
    }

    /// Send the worker thread the function `f` to run.
    pub(super) fn send_function(
        &mut self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> io::Result<()> {
        self.channel.try_send(Control::Run(f))
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<Result<(), rt::Error>> {
        self.handle.join()
    }
}

/// Worker that runs thread-local and thread-safe actors and futurers, and
/// holds and manages everything that is required to run them.
pub(crate) struct Worker {
    /// Internals of the runtime, shared with zero or more [`RuntimeRef`]s.
    internals: Rc<RuntimeInternals>,
    /// Mio events container.
    events: Events,
    /// Receiving side of the channel for waker events, see the
    /// [`rt::local::waker`] module for the implementation.
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

impl Worker {
    /// Setup the worker. Must be called on the worker thread.
    fn setup(
        setup: WorkerSetup,
        mut receiver: rt::channel::Receiver<Control>,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> Result<Worker, Error> {
        let timing = trace::start(&trace_log);

        let cpu = if auto_cpu_affinity {
            set_cpu_affinity(setup.id)
        } else {
            None
        };

        // Register the shared poll intance.
        let poll = setup.poll;
        trace!(worker_id = setup.id.get(); "registering I/O uring");
        let ring = setup.ring;
        let ring_fd = ring.as_fd().as_raw_fd();
        poll.registry()
            .register(&mut SourceFd(&ring_fd), RING, Interest::READABLE)
            .map_err(Error::Init)?;
        trace!(worker_id = setup.id.get(); "registering shared poll");
        shared_internals
            .register_worker_poll(poll.registry(), SHARED_POLL)
            .map_err(Error::Init)?;
        // Register the channel to the coordinator.
        trace!(worker_id = setup.id.get(); "registering communication channel");
        receiver
            .register(poll.registry(), COMMS)
            .map_err(Error::Init)?;

        // Finally create all the runtime internals.
        let internals = RuntimeInternals::new(
            setup.id,
            shared_internals,
            setup.waker_id,
            poll,
            ring,
            cpu,
            trace_log,
        );
        let mut worker = Worker {
            internals: Rc::new(internals),
            events: Events::with_capacity(128),
            waker_events: setup.waker_events,
            channel: receiver,
            started: false,
        };

        trace::finish_rt(
            worker.trace_log().as_mut(),
            timing,
            "Initialising the worker thread",
            &[],
        );
        Ok(worker)
    }

    /// Create a new local `Runtime` for testing.
    ///
    /// Used in the [`crate::test`] module.
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn new_test(
        shared_internals: Arc<shared::RuntimeInternals>,
        mut receiver: rt::channel::Receiver<Control>,
    ) -> io::Result<Worker> {
        let poll = Poll::new()?;
        // TODO: configure ring.
        let ring = a10::Ring::new(512)?;

        // TODO: this channel will grow unbounded as the waker implementation
        // sends pids into it.
        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(poll.registry(), WAKER)?;
        let waker_id = waker::init(waker, waker_sender);

        receiver.register(poll.registry(), COMMS)?;

        let id = NonZeroUsize::new(usize::MAX).unwrap();
        let internals =
            RuntimeInternals::new(id, shared_internals, waker_id, poll, ring, None, None);
        Ok(Worker {
            internals: Rc::new(internals),
            events: Events::with_capacity(16),
            waker_events,
            channel: receiver,
            started: false,
        })
    }

    /// Run the worker.
    pub(crate) fn run(mut self) -> Result<(), Error> {
        debug!(worker_id = self.internals.id.get(); "starting worker");
        // Runtime reference used in running the processes.
        let mut runtime_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there are no processes to run.
            trace!(worker_id = self.internals.id.get(); "running processes");
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
                debug!(worker_id = self.internals.id.get(); "no processes to run, stopping worker");
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
                let timing = trace::start(&*self.internals.trace_log.borrow());
                let pid = process.as_ref().id();
                let name = process.as_ref().name();
                match process.as_mut().run(runtime_ref) {
                    ProcessResult::Complete => {
                        // Don't want to panic when dropping the process.
                        drop(catch_unwind(AssertUnwindSafe(move || drop(process))));
                    }
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
        trace!(worker_id = self.internals.id.get(); "polling event sources to schedule processes");
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
        let mut check_ring = false;
        let mut amount = 0;
        for event in self.events.iter() {
            trace!(worker_id = self.internals.id.get(); "got OS event: {event:?}");
            match event.token() {
                WAKER => { /* Need to wake up to handle user space events. */ }
                COMMS => check_comms = true,
                SHARED_POLL => check_shared_poll = true,
                RING => check_ring = true,
                token => {
                    let pid = ProcessId::from(token);
                    trace!(
                        worker_id = self.internals.id.get(), pid = pid.0;
                        "scheduling local process based on OS event",
                    );
                    scheduler.mark_ready(pid);
                    amount += 1;
                }
            }
        }

        if check_ring {
            self.internals
                .ring
                .borrow_mut()
                .poll(Some(Duration::ZERO))
                .map_err(Error::Polling)?;
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
        trace!(worker_id = self.internals.id.get(); "polling shared OS events");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut amount = 0;
        if self.internals.shared.try_poll(&mut self.events)? {
            for event in self.events.iter() {
                trace!(worker_id = self.internals.id.get(); "got shared OS event: {event:?}");
                let pid = ProcessId::from(event.token());
                trace!(
                    worker_id = self.internals.id.get(), pid = pid.0;
                    "scheduling shared process based on OS event",
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

        let mut scheduler = self.internals.scheduler.borrow_mut();
        let mut amount: usize = 0;
        for pid in self.internals.timers.borrow_mut().deadlines(now) {
            trace!(worker_id = self.internals.id.get(), pid = pid.0; "expiring timer for local process");
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
        trace!(worker_id = self.internals.id.get(); "polling shared timers");
        let timing = trace::start(&*self.internals.trace_log.borrow());

        let mut amount: usize = 0;
        while let Some(pid) = self.internals.shared.remove_next_deadline(now) {
            trace!(worker_id = self.internals.id.get(), pid = pid.0; "expiring timer for shared process");
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
        let mut timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is non zero.
        let marked_polling = if timeout.map_or(true, |t| !t.is_zero()) {
            waker::mark_polling(self.internals.waker_id, true);
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

        trace!(worker_id = self.internals.id.get(), timeout = as_debug!(timeout); "polling OS events");
        let res = self
            .internals
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout);

        if marked_polling {
            waker::mark_polling(self.internals.waker_id, false);
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
        trace!(worker_id = self.internals.id.get(); "processing coordinator messages");
        let timing = trace::start(&*self.internals.trace_log.borrow());
        while let Some(msg) = self.channel.try_recv().map_err(Error::RecvMsg)? {
            match msg {
                Control::Started => self.started = true,
                Control::Signal(signal) => {
                    if let Signal::User2 = signal {
                        self.log_metrics();
                    }

                    self.relay_signal(signal)?;
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
        trace!(worker_id = self.internals.id.get(), signal = as_debug!(signal); "received process signal");

        let mut receivers = self.internals.signal_receivers.borrow_mut();
        receivers.remove_disconnected();
        let res = match receivers.try_send(signal, Delivery::ToAll) {
            Err(SendError) if signal.should_stop() => Err(Error::ProcessInterrupted),
            Ok(()) | Err(SendError) => Ok(()),
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
        trace!(worker_id = self.internals.id.get(); "running user function");
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

    /// Gather metrics about the runtime internals.
    fn log_metrics(&self) {
        let shared = &*self.internals;
        let timing = trace::start(&*shared.trace_log.borrow());
        let trace_metrics = shared.trace_log.borrow().as_ref().map(trace::Log::metrics);
        let scheduler = shared.scheduler.borrow();
        // NOTE: need mutable access to timers due to `Timers::next`.
        let mut timers = shared.timers.borrow_mut();
        info!(
            target: "metrics",
            worker_id = shared.id.get(),
            cpu_affinity = shared.cpu,
            scheduler_ready = scheduler.ready(),
            scheduler_inactive = scheduler.inactive(),
            timers_total = timers.len(),
            timers_next = as_debug!(timers.next_timer()),
            process_signal_receivers = shared.signal_receivers.borrow().len(),
            cpu_time = as_debug!(cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID)),
            trace_counter = trace_metrics.map_or(0, |m| m.counter);
            "worker metrics",
        );
        trace::finish_rt(
            shared.trace_log.borrow_mut().as_mut(),
            timing,
            "Printing runtime metrics",
            &[],
        );
    }

    /// Create a new reference to this runtime.
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
            Init(err) => write!(f, "error initialising local runtime: {err}"),
            Polling(err) => write!(f, "error polling OS: {err}"),
            RecvMsg(err) => write!(f, "error receiving message(s): {err}"),
            ProcessInterrupted => write!(
                f,
                "received process signal, but no receivers for it: stopping runtime"
            ),
            UserFunction(err) => write!(f, "error running user function: {err}"),
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
