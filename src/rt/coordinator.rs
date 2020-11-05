//! Coordinator thread code.

use std::cmp::max;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{fmt, io};

use crossbeam_channel::Receiver;
use log::{debug, trace, warn};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::rt::process::ProcessId;
use crate::rt::scheduler::Scheduler;
use crate::rt::waker::{self, WakerId};
use crate::rt::{
    self, SharedRuntimeInternal, Signal, SyncWorker, Timers, Worker, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};
use crate::ActorRef;

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

/// Tokens used to receive events.
const SIGNAL: Token = Token(usize::max_value());
pub(super) const WAKER: Token = Token(usize::max_value() - 1);

pub(super) struct Coordinator {
    poll: Poll,
    waker_id: WakerId,
    waker_events: Receiver<ProcessId>,
    scheduler: Scheduler,
    timers: Arc<Mutex<Timers>>,
}

impl Coordinator {
    /// Initialise the `Coordinator` thread.
    pub(super) fn init() -> Result<(Coordinator, Arc<SharedRuntimeInternal>), Error> {
        let poll = Poll::new().map_err(Error::Init)?;
        let registry = poll.registry().try_clone().map_err(Error::Init)?;

        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(&registry, WAKER).map_err(Error::Init)?;
        let waker_id = waker::init(waker, waker_sender);
        let scheduler = Scheduler::new();
        let timers = Arc::new(Mutex::new(Timers::new()));

        let shared_internals =
            SharedRuntimeInternal::new(waker_id, scheduler.create_ref(), registry, timers.clone());
        let coordinator = Coordinator {
            poll,
            waker_events,
            waker_id,
            scheduler,
            timers,
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
        mut signal_refs: Vec<ActorRef<Signal>>,
    ) -> Result<(), rt::Error<E>> {
        debug_assert!(workers.is_sorted_by_key(|w| w.id()));

        let registry = self.poll.registry();
        let mut signals = setup_signals(&registry)
            .map_err(|err| rt::Error::coordinator(Error::SetupSignals(err)))?;
        register_workers(&registry, &mut workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringWorkers(err)))?;
        register_sync_workers(&registry, &mut sync_workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringSyncActors(err)))?;

        let mut events = Events::with_capacity(16);
        let mut workers_waker_idx = 0;
        loop {
            self.poll(&mut events)
                .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;

            let mut wake_workers = 0;
            for event in events.iter() {
                trace!("event: {:?}", event);
                match event.token() {
                    SIGNAL => relay_signals(&mut signals, &mut workers, &mut signal_refs)
                        .map_err(|err| rt::Error::coordinator(Error::SignalRelay(err)))?,
                    // We always check for waker events below.
                    WAKER => {}
                    token if token.0 < SYNC_WORKER_ID_START => {
                        handle_worker_event(&mut workers, event)?
                    }
                    token if token.0 <= SYNC_WORKER_ID_END => {
                        handle_sync_worker_event(&mut sync_workers, event)?
                    }
                    token => {
                        wake_workers += 1;
                        let pid = token.into();
                        trace!("waking thread-safe actor: pid={}", pid);
                        self.scheduler.mark_ready(pid);
                    }
                }
            }

            trace!("polling timers");
            for pid in self.timers.lock().unwrap().deadlines() {
                trace!("waking thread-safe actor: pid={}", pid);
                wake_workers += 1;
                self.scheduler.mark_ready(pid);
            }

            trace!("polling wake-up events");
            for pid in self.waker_events.try_iter() {
                trace!("waking thread-safe actor: pid={}", pid);
                wake_workers += 1;
                if pid.0 != WAKER.0 {
                    self.scheduler.mark_ready(pid);
                }
            }
            // In case the worker threads are polling we need to wake them up.
            // TODO: optimise this.
            if wake_workers > 0 {
                trace!("waking worker threads");
                // To prevent the Thundering herd problem [1] we don't wake all
                // workers, only enough worker threads to handle all events.
                //
                // [1]: https://en.wikipedia.org/wiki/Thundering_herd_problem
                let wake_workers = max(wake_workers, workers.len());
                let workers_to_wake = workers
                    .iter_mut()
                    .skip(workers_waker_idx)
                    .take(wake_workers);
                for worker in workers_to_wake {
                    // Can't deal with the error.
                    trace!("waking worker thread: id={}", worker.id());
                    if let Err(err) = worker.wake() {
                        warn!("error waking working thread: {}: id={}", err, worker.id());
                    }
                }
                workers_waker_idx = (workers_waker_idx + wake_workers) % workers.len();
            }

            if workers.is_empty() && sync_workers.is_empty() {
                return Ok(());
            }
        }
    }

    fn poll(&mut self, events: &mut Events) -> io::Result<()> {
        trace!("polling event sources");
        let timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is not zero.
        let mark_waker = if !is_zero(timeout) {
            waker::mark_polling(self.waker_id, true);
            true
        } else {
            false
        };

        let res = self.poll.poll(events, timeout);

        if mark_waker {
            waker::mark_polling(self.waker_id, false);
        }

        res
    }

    /// Determine the timeout to be used in `Poll::poll`.
    fn determine_timeout(&self) -> Option<Duration> {
        if let Some(deadline) = self.timers.lock().unwrap().next_deadline() {
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
    workers
        .iter_mut()
        .map(|worker| {
            let id = worker.id();
            trace!("registering worker thread: id={}", id);
            worker.register(&registry, Token(id))
        })
        .collect()
}

/// Register all `sync_workers`' sending end of the pipe with `registry`.
fn register_sync_workers(registry: &Registry, sync_workers: &mut [SyncWorker]) -> io::Result<()> {
    sync_workers
        .iter_mut()
        .map(|worker| {
            let id = worker.id();
            trace!("registering sync actor worker thread: id={}", id);
            registry.register(worker, Token(id), Interest::WRITABLE)
        })
        .collect()
}

/// Relay all signals receive from `signals` to the `workers` and
/// `sync_workers`.
fn relay_signals<E>(
    signals: &mut Signals,
    workers: &mut [Worker<E>],
    signal_refs: &mut Vec<ActorRef<Signal>>,
) -> io::Result<()> {
    while let Some(signal) = signals.receive()? {
        debug!("received signal on coordinator: signal={:?}", signal);

        let signal = Signal::from_mio(signal);
        for worker in workers.iter_mut() {
            worker.send_signal(signal)?;
        }
        for actor_ref in signal_refs.iter() {
            if let Err(err) = actor_ref.send(signal) {
                warn!("failed to send process signal to actor: {}", err);
            }
        }
    }
    Ok(())
}

/// Handle an `event` for a worker.
fn handle_worker_event<E>(workers: &mut Vec<Worker<E>>, event: &Event) -> Result<(), rt::Error<E>> {
    if let Ok(i) = workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the
            // worker has shut down.
            let worker = workers.remove(i);
            debug!("worker thread stopped: id={}", worker.id());

            worker
                .join()
                .map_err(rt::Error::worker_panic)
                .and_then(|res| res)
        } else {
            // Sporadic event, we can ignore it.
            Ok(())
        }
    } else {
        // Sporadic event, we can ignore it.
        Ok(())
    }
}

/// Handle an `event` for a sync actor worker.
fn handle_sync_worker_event<E>(
    sync_workers: &mut Vec<SyncWorker>,
    event: &Event,
) -> Result<(), rt::Error<E>> {
    if let Ok(i) = sync_workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the
            // worker has shut down.
            let sync_worker = sync_workers.remove(i);
            debug!("sync actor worker thread stopped: id={}", sync_worker.id());

            sync_worker.join().map_err(rt::Error::sync_actor_panic)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}
