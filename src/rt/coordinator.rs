//! Coordinator thread code.

use std::sync::{Arc, Mutex};
use std::{fmt, io};

use log::{debug, trace, warn};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::actor_ref::{ActorGroup, Delivery};
use crate::rt::shared::{waker, Scheduler};
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{
    self, shared, worker, Signal, SyncWorker, Timers, Worker, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};
use crate::trace;

/// Token used to receive process signals.
const SIGNAL: Token = Token(usize::MAX);
/// Token used to wake-up the coordinator thread.
pub(super) const WAKER: Token = Token(usize::MAX - 1);

/// Stream id for the coordinator.
pub(super) const TRACE_ID: u32 = 0;

#[derive(Debug)]
pub(super) struct Coordinator {
    /// OS poll, used to poll the status of the (sync) worker threads and
    /// process signals.
    poll: Poll,
    /// Signal notifications.
    signals: Signals,
    /// Internals shared between the coordinator and workers.
    internals: Arc<shared::RuntimeInternals>,
}

impl Coordinator {
    /// Initialise the `Coordinator` thread.
    pub(super) fn init(worker_wakers: Box<[&'static ThreadWaker]>) -> io::Result<Coordinator> {
        let poll = Poll::new()?;
        let registry = poll.registry().try_clone()?;
        let scheduler = Scheduler::new();
        let timers = Mutex::new(Timers::new());
        // NOTE: on Linux this MUST be created before starting the worker
        // threads.
        let signals = setup_signals(&registry)?;
        let internals = Arc::new_cyclic(|shared_internals| {
            let waker_id = waker::init(shared_internals.clone());
            shared::RuntimeInternals::new(waker_id, worker_wakers, scheduler, registry, timers)
        });
        Ok(Coordinator {
            poll,
            signals,
            internals,
        })
    }

    /// Get access to the shared runtime internals.
    pub(super) const fn shared_internals(&self) -> &Arc<shared::RuntimeInternals> {
        &self.internals
    }

    /// Run the coordinator.
    ///
    /// # Notes
    ///
    /// `workers` must be sorted based on `id`.
    pub(super) fn run(
        mut self,
        mut workers: Vec<Worker>,
        mut sync_workers: Vec<SyncWorker>,
        mut signal_refs: ActorGroup<Signal>,
        mut trace_log: Option<trace::Log>,
    ) -> Result<(), rt::Error> {
        debug_assert!(workers.is_sorted_by_key(Worker::id));
        debug_assert!(sync_workers.is_sorted_by_key(SyncWorker::id));

        // Register various sources of OS events that need to wake us from
        // polling events.
        let timing = trace::start(&trace_log);
        let registry = self.poll.registry();
        register_workers(registry, &mut workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringWorkers(err)))?;
        register_sync_workers(registry, &mut sync_workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringSyncActors(err)))?;

        // It could be that before we're able to register the (sync) worker
        // above they've already completed. On macOS and Linux this will still
        // create an kqueue/epoll event, even if the other side of the Unix pipe
        // was already disconnected before registering it with `Poll`.
        // However this doesn't seem to be the case on FreeBSD, so we explicitly
        // check if the (sync) workers are still alive *after* we registered
        // them.
        check_sync_worker_alive(&mut sync_workers)?;
        trace::finish(
            &mut trace_log,
            timing,
            "Initialising the coordinator thread",
            &[],
        );

        // Signal to all worker threads the runtime was started. See
        // RunningRuntime::started why this is needed.
        for worker in &mut workers {
            worker
                .send_runtime_started()
                .map_err(|err| rt::Error::coordinator(Error::SendingStartSignal(err)))?;
        }

        let mut events = Events::with_capacity(16);
        loop {
            let timing = trace::start(&trace_log);
            // Process OS events.
            self.poll
                .poll(&mut events, None)
                .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
            trace::finish(&mut trace_log, timing, "Polling for OS events", &[]);

            let timing = trace::start(&trace_log);
            let mut wake_workers = 0; // Counter for how many workers to wake.
            for event in events.iter() {
                trace!("event: {:?}", event);

                match event.token() {
                    SIGNAL => {
                        let timing = trace::start(&trace_log);
                        relay_signals(&mut self.signals, &mut workers, &mut signal_refs)
                            .map_err(|err| rt::Error::coordinator(Error::SignalRelay(err)))?;
                        trace::finish(&mut trace_log, timing, "Relaying process signal(s)", &[]);
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

            // In case the worker threads are polling we need to wake them up.
            if wake_workers > 0 {
                let timing = trace::start(&trace_log);
                self.internals.wake_workers(wake_workers);
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
fn register_workers(registry: &Registry, workers: &mut [Worker]) -> io::Result<()> {
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

/// Checks all `sync_workers` if they're alive and removes any that have
/// stopped.
fn check_sync_worker_alive(sync_workers: &mut Vec<SyncWorker>) -> Result<(), rt::Error> {
    sync_workers
        .drain_filter(|sync_worker| !sync_worker.is_alive())
        .try_for_each(|sync_worker| {
            debug!("sync actor worker thread stopped: id={}", sync_worker.id());
            sync_worker.join().map_err(rt::Error::sync_actor_panic)
        })
}

/// Relay all signals receive from `signals` to the `workers` and
/// `sync_workers`.
fn relay_signals(
    signals: &mut Signals,
    workers: &mut [Worker],
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
fn handle_worker_event(workers: &mut Vec<Worker>, event: &Event) -> Result<(), rt::Error> {
    if let Ok(i) = workers.binary_search_by_key(&event.token().0, Worker::id) {
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
fn handle_sync_worker_event(
    sync_workers: &mut Vec<SyncWorker>,
    event: &Event,
) -> Result<(), rt::Error> {
    if let Ok(i) = sync_workers.binary_search_by_key(&event.token().0, SyncWorker::id) {
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
    /// Error in [`register_workers`].
    RegisteringWorkers(io::Error),
    /// Error in [`register_sync_workers`].
    RegisteringSyncActors(io::Error),
    /// Error polling [`mio::Poll`].
    Polling(io::Error),
    /// Error sending start signal to worker.
    SendingStartSignal(io::Error),
    /// Error relaying process signal.
    SignalRelay(io::Error),
    /// Error sending function to worker.
    SendingFunc(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            RegisteringWorkers(err) => write!(f, "error registering worker threads: {}", err),
            RegisteringSyncActors(err) => {
                write!(f, "error registering synchronous actor threads: {}", err)
            }
            Polling(err) => write!(f, "error polling for events: {}", err),
            SendingStartSignal(err) => write!(f, "error sending start signal to worker: {}", err),
            SignalRelay(err) => write!(f, "error relaying process signal: {}", err),
            SendingFunc(err) => write!(f, "error sending function to worker: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            RegisteringWorkers(ref err)
            | RegisteringSyncActors(ref err)
            | Polling(ref err)
            | SendingStartSignal(ref err)
            | SignalRelay(ref err)
            | SendingFunc(ref err) => Some(err),
        }
    }
}
