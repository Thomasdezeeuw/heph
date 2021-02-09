//! Coordinator thread code.

use std::cmp::min;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{fmt, io};

use crossbeam_channel::Receiver;
use log::{debug, trace, warn};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::actor_ref::{ActorGroup, Delivery};
use crate::rt::process::ProcessId;
use crate::rt::shared::Scheduler;
use crate::rt::{
    self, shared, waker, worker, Signal, SyncWorker, Timers, Worker, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};
use crate::trace;

/// Token used to receive process signals.
const SIGNAL: Token = Token(usize::max_value());
/// Token used to wake-up the coordinator thread.
pub(super) const WAKER: Token = Token(usize::max_value() - 1);

/// Stream id for the coordinator.
pub(super) const TRACE_ID: u32 = 0;

pub(super) struct Coordinator {
    /// OS poll, used to poll the status of the (sync) worker threads and
    /// process signals.
    poll: Poll,
    /// Receiving end of the wake-up events used in the `rt::waker` mechanism.
    waker_events: Receiver<ProcessId>,
    /// Internals shared between the coordinator and workers.
    internals: Arc<shared::RuntimeInternals>,
}

impl Coordinator {
    /// Initialise the `Coordinator` thread.
    pub(super) fn init() -> Result<(Coordinator, Arc<shared::RuntimeInternals>), Error> {
        let poll = Poll::new().map_err(Error::Init)?;
        let registry = poll.registry().try_clone().map_err(Error::Init)?;

        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(&registry, WAKER).map_err(Error::Init)?;
        let waker_id = waker::init(waker, waker_sender);
        let scheduler = Scheduler::new();
        let timers = Mutex::new(Timers::new());

        let shared_internals = shared::RuntimeInternals::new(waker_id, scheduler, registry, timers);
        let coordinator = Coordinator {
            poll,
            waker_events,
            internals: shared_internals.clone(),
        };

        Ok((coordinator, shared_internals))
    }

    /// Run the coordinator.
    ///
    /// # Notes
    ///
    /// `workers` must be sorted based on `id`.
    pub(super) fn run<E>(
        mut self,
        mut workers: Vec<Worker<E>>,
        mut sync_workers: Vec<SyncWorker>,
        mut signal_refs: ActorGroup<Signal>,
        mut trace_log: Option<trace::Log>,
    ) -> Result<(), rt::Error<E>> {
        debug_assert!(workers.is_sorted_by_key(|w| w.id()));

        // Register various sources of OS events that need to wake us from
        // polling events.
        let timing = trace::start(&trace_log);
        let registry = self.poll.registry();
        let mut signals = setup_signals(&registry)
            .map_err(|err| rt::Error::coordinator(Error::SetupSignals(err)))?;
        register_workers(&registry, &mut workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringWorkers(err)))?;
        register_sync_workers(&registry, &mut sync_workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringSyncActors(err)))?;

        // It could be that before we're able to register the (sync) worker
        // above they've already completed. On macOS and Linux this will still
        // create an kqueue/epoll event, even if the other side of the Unix pipe
        // was already disconnected before registering it with `Poll`.
        // However this doesn't seem to be the case on FreeBSD, so we explicitly
        // check if the (sync) workers are still alive *after* we registered
        // them.
        check_worker_alive(&mut workers)?;
        check_sync_worker_alive(&mut sync_workers)?;
        trace::finish(
            &mut trace_log,
            timing,
            "Initialising the coordinator thread",
            &[],
        );

        // Index of the last worker we waked, must never be larger then
        // `worker.len()` (which changes throughout the loop).
        let mut workers_waker_idx = 0;
        let mut events = Events::with_capacity(16);
        loop {
            // Counter for how many workers to wake.
            let mut wake_workers = 0;

            let timing = trace::start(&trace_log);
            // Process OS events.
            self.poll(&mut events)
                .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
            trace::finish(&mut trace_log, timing, "Polling for OS events", &[]);

            let timing = trace::start(&trace_log);
            for event in events.iter() {
                trace!("event: {:?}", event);
                match event.token() {
                    SIGNAL => {
                        let timing = trace::start(&trace_log);
                        relay_signals(&mut signals, &mut workers, &mut signal_refs)
                            .map_err(|err| rt::Error::coordinator(Error::SignalRelay(err)))?;
                        trace::finish(&mut trace_log, timing, "Relaying process signal", &[]);
                    }
                    // We always check for waker events below.
                    WAKER => {}
                    token if token.0 < SYNC_WORKER_ID_START => {
                        let timing = trace::start(&trace_log);
                        handle_worker_event(&mut workers, event)?;
                        trace::finish(&mut trace_log, timing, "Processing worker event", &[]);
                    }
                    token if token.0 <= SYNC_WORKER_ID_END => {
                        let timing = trace::start(&trace_log);
                        handle_sync_worker_event(&mut sync_workers, event)?;
                        trace::finish(&mut trace_log, timing, "Processing sync worker event", &[]);
                    }
                    token => {
                        let timing = trace::start(&trace_log);
                        let pid = token.into();
                        trace!("waking thread-safe actor: pid={}", pid);
                        self.internals.mark_ready(pid);
                        wake_workers += 1;
                        trace::finish(
                            &mut trace_log,
                            timing,
                            "Scheduling thread-safe process",
                            &[],
                        );
                    }
                }
            }
            trace::finish(&mut trace_log, timing, "Handling OS events", &[]);

            trace!("polling wake-up events");
            let timing = trace::start(&trace_log);
            for pid in self.waker_events.try_iter() {
                trace!("waking thread-safe actor: pid={}", pid);
                if pid.0 != WAKER.0 {
                    self.internals.mark_ready(pid);
                    wake_workers += 1;
                }
            }
            trace::finish(
                &mut trace_log,
                timing,
                "Scheduling thread-safe processes based on wake-up events",
                &[],
            );

            trace!("polling timers");
            let timing = trace::start(&trace_log);
            for pid in self.internals.timers().lock().unwrap().deadlines() {
                trace!("waking thread-safe actor: pid={}", pid);
                self.internals.mark_ready(pid);
                wake_workers += 1;
            }
            trace::finish(
                &mut trace_log,
                timing,
                "Scheduling thread-safe processes based on timers",
                &[],
            );

            // In case the worker threads are polling we need to wake them up.
            if wake_workers > 0 {
                trace!("waking worker threads");
                let timing = trace::start(&trace_log);
                // To prevent the Thundering herd problem [1] we don't wake all
                // workers, only enough worker threads to handle all events. To
                // spread the workload (somewhat more) evenly we wake the
                // workers in a Round-Robin [2] fashion.
                //
                // [1]: https://en.wikipedia.org/wiki/Thundering_herd_problem
                // [2]: https://en.wikipedia.org/wiki/Round-robin_scheduling
                wake_workers = min(wake_workers, workers.len());
                let (wake_second, wake_first) = workers.split_at_mut(workers_waker_idx);
                let workers_to_wake = wake_first.iter_mut().chain(wake_second.iter_mut());
                let mut wakes_left = wake_workers;
                for worker in workers_to_wake {
                    trace!("waking worker: id={}", worker.id());
                    match worker.wake() {
                        Ok(true) => {
                            wakes_left -= 1;
                            if wakes_left == 0 {
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(err) => warn!("error waking worker: {}: id={}", err, worker.id()),
                    }
                }
                workers_waker_idx = (workers_waker_idx + wake_workers) % workers.len();
                trace::finish(
                    &mut trace_log,
                    timing,
                    "Waking worker threads",
                    &[("amount", &wake_workers)],
                );
            }

            // Once all (sync) worker threads are done running we can return.
            if workers.is_empty() && sync_workers.is_empty() {
                return Ok(());
            }
        }
    }

    fn poll(&mut self, events: &mut Events) -> io::Result<()> {
        let timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is not zero.
        let mark_waker = if !is_zero(timeout) {
            waker::mark_polling(self.internals.coordinator_id(), true);
            true
        } else {
            false
        };

        trace!("polling OS: timeout={:?}", timeout);
        let res = self.poll.poll(events, timeout);

        if mark_waker {
            waker::mark_polling(self.internals.coordinator_id(), false);
        }

        res
    }

    /// Determine the timeout to be used in `Poll::poll`.
    fn determine_timeout(&self) -> Option<Duration> {
        if let Some(deadline) = self.internals.timers().lock().unwrap().next_deadline() {
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
}

/// Returns `true` is timeout is `Some(Duration::from_nanos(0))`.
fn is_zero(timeout: Option<Duration>) -> bool {
    timeout.map(|t| t.is_zero()).unwrap_or(false)
}

/// Setup a new `Signals` instance, registering it with `registry`.
fn setup_signals(registry: &Registry) -> io::Result<Signals> {
    let signals = SignalSet::all();
    trace!("setting up signals handling: signals={:?}", signals);
    Signals::new(signals).and_then(|mut signals| {
        registry
            .register(&mut signals, SIGNAL, Interest::READABLE)
            .map(|()| signals)
    })
}

/// Register all `workers`' sending end of the pipe with `registry`.
fn register_workers<E>(registry: &Registry, workers: &mut [Worker<E>]) -> io::Result<()> {
    workers.iter_mut().try_for_each(|worker| {
        trace!("registering worker thread: id={}", worker.id());
        worker.register(registry)
    })
}

/// Register all `sync_workers`' sending end of the pipe with `registry`.
fn register_sync_workers(registry: &Registry, sync_workers: &mut [SyncWorker]) -> io::Result<()> {
    sync_workers.iter_mut().try_for_each(|worker| {
        trace!("registering sync actor worker thread: id={}", worker.id());
        worker.register(registry)
    })
}

/// Checks all `workers` if they're alive and removes any that have stopped.
fn check_worker_alive<E>(workers: &mut Vec<Worker<E>>) -> Result<(), rt::Error<E>> {
    workers
        .drain_filter(|worker| !worker.is_alive())
        .try_for_each(|worker| {
            debug!("worker thread stopped: id={}", worker.id());
            worker
                .join()
                .map_err(rt::Error::worker_panic)
                .and_then(|res| res)
        })
}

/// Checks all `sync_workers` if they're alive and removes any that have
/// stopped.
fn check_sync_worker_alive<E>(sync_workers: &mut Vec<SyncWorker>) -> Result<(), rt::Error<E>> {
    sync_workers
        .drain_filter(|sync_worker| !sync_worker.is_alive())
        .try_for_each(|sync_worker| {
            debug!("sync actor worker thread stopped: id={}", sync_worker.id());
            sync_worker.join().map_err(rt::Error::sync_actor_panic)
        })
}

/// Relay all signals receive from `signals` to the `workers` and
/// `sync_workers`.
fn relay_signals<E>(
    signals: &mut Signals,
    workers: &mut [Worker<E>],
    signal_refs: &mut ActorGroup<Signal>,
) -> io::Result<()> {
    signal_refs.remove_disconnected();

    while let Some(signal) = signals.receive()? {
        debug!("received signal on coordinator: signal={:?}", signal);
        let signal = Signal::from_mio(signal);
        for worker in workers.iter_mut() {
            worker.send_signal(signal)?;
        }
        if !signal_refs.is_empty() {
            if let Err(err) = signal_refs.try_send(signal, Delivery::ToAll) {
                warn!("failed to send process signal: {}", err);
            }
        }
    }
    Ok(())
}

/// Handle an `event` for a worker.
fn handle_worker_event<E>(workers: &mut Vec<Worker<E>>, event: &Event) -> Result<(), rt::Error<E>> {
    if let Ok(i) = workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the worker has
            // shut down.
            let worker = workers.remove(i);
            debug!("worker thread stopped: id={}", worker.id());
            worker
                .join()
                .map_err(rt::Error::worker_panic)
                .and_then(|res| res)?;
        } else if event.is_readable() {
            let worker = &mut workers[i];
            debug!("handling worker messages: id={}", worker.id());
            worker
                .handle_messages()
                .map_err(|err| rt::Error::worker(worker::Error::RecvMsg(err)))?;
        }
    }
    Ok(())
}

/// Handle an `event` for a sync actor worker.
fn handle_sync_worker_event<E>(
    sync_workers: &mut Vec<SyncWorker>,
    event: &Event,
) -> Result<(), rt::Error<E>> {
    if let Ok(i) = sync_workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the worker has
            // shut down.
            let sync_worker = sync_workers.remove(i);
            debug!("sync actor worker thread stopped: id={}", sync_worker.id());
            sync_worker.join().map_err(rt::Error::sync_actor_panic)?;
        }
    }
    Ok(())
}

/// Error running the [`Coordinator`].
#[derive(Debug)]
pub(super) enum Error {
    /// Error in [`Coordinator::init`].
    Init(io::Error),
    /// Error in [`setup_signals`].
    SetupSignals(io::Error),
    /// Error in [`register_workers`].
    RegisteringWorkers(io::Error),
    /// Error in [`register_sync_workers`].
    RegisteringSyncActors(io::Error),
    /// Error polling [`mio::Poll`].
    Polling(io::Error),
    /// Error relaying process signal.
    SignalRelay(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            Init(err) => write!(f, "error initialising coordinator: {}", err),
            SetupSignals(err) => write!(f, "error setting up process signal handling: {}", err),
            RegisteringWorkers(err) => write!(f, "error registering worker threads: {}", err),
            RegisteringSyncActors(err) => {
                write!(f, "error registering synchronous actor threads: {}", err)
            }
            Polling(err) => write!(f, "error polling for events: {}", err),
            SignalRelay(err) => write!(f, "error relay process signal: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            Init(ref err)
            | SetupSignals(ref err)
            | RegisteringWorkers(ref err)
            | RegisteringSyncActors(ref err)
            | Polling(ref err)
            | SignalRelay(ref err) => Some(err),
        }
    }
}
