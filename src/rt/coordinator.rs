//! Coordinator thread code.

use std::sync::Arc;
use std::{fmt, io};

use log::{debug, error, info, trace};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::actor_ref::{ActorGroup, Delivery};
use crate::rt::shared::waker;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{
    self, shared, Signal, SyncWorker, Worker, SYNC_WORKER_ID_END, SYNC_WORKER_ID_START,
};
use crate::trace;

/// Token used to receive process signals.
const SIGNAL: Token = Token(usize::MAX);

#[derive(Debug)]
pub(super) struct Coordinator {
    /// OS poll, used to poll the status of the (sync) worker threads and
    /// process `signals`.
    poll: Poll,
    /// Signal notifications.
    signals: Signals,
    /// Internals shared between the coordinator and workers.
    internals: Arc<shared::RuntimeInternals>,
}

/// Metrics for [`Coordinator`].
#[derive(Debug)]
struct Metrics<'l> {
    worker_threads: usize,
    sync_actors: usize,
    shared: shared::Metrics,
    process_signals: SignalSet,
    process_signal_receivers: usize,
    trace_log: Option<trace::CoordinatorMetrics<'l>>,
}

impl Coordinator {
    /// Initialise the `Coordinator`.
    ///
    /// # Notes
    ///
    /// This must be called before creating the worker threads to properly catch
    /// process signals.
    pub(super) fn init(
        worker_wakers: Box<[&'static ThreadWaker]>,
        trace_log: Option<Arc<trace::SharedLog>>,
    ) -> io::Result<Coordinator> {
        let poll = Poll::new()?;
        // NOTE: on Linux this MUST be created before starting the worker
        // threads.
        let signals = setup_signals(poll.registry())?;

        let setup = shared::RuntimeInternals::setup()?;
        let internals = Arc::new_cyclic(|shared_internals| {
            let waker_id = waker::init(shared_internals.clone());
            setup.complete(waker_id, worker_wakers, trace_log)
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
    /// `workers` and `sync_workers` must be sorted based on `id`.
    pub(super) fn run(
        mut self,
        mut workers: Vec<Worker>,
        mut sync_workers: Vec<SyncWorker>,
        mut signal_refs: ActorGroup<Signal>,
        mut trace_log: Option<trace::CoordinatorLog>,
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
        // It could be that before we're able to register the sync workers above
        // they've already completed. On macOS and Linux this will still create
        // an kqueue/epoll event, even if the other side of the Unix pipe was
        // already disconnected before registering it with `Poll`.
        // However this doesn't seem to be the case on FreeBSD, so we explicitly
        // check if the sync workers are still alive *after* we registered them.
        check_sync_worker_alive(&mut sync_workers)?;
        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Initialising the coordinator thread",
            &[],
        );

        // Signal to all worker threads the runtime was started. See
        // `local::Runtime.started` why this is needed.
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
            trace::finish_rt(trace_log.as_mut(), timing, "Polling for OS events", &[]);

            let timing = trace::start(&trace_log);
            for event in events.iter() {
                trace!("got OS event: {:?}", event);

                match event.token() {
                    SIGNAL => {
                        let timing = trace::start(&trace_log);
                        let log_metrics =
                            relay_signals(&mut self.signals, &mut workers, &mut signal_refs);
                        trace::finish_rt(
                            trace_log.as_mut(),
                            timing,
                            "Relaying process signal(s)",
                            &[],
                        );

                        if log_metrics {
                            let timing = trace::start(&trace_log);
                            let metrics =
                                self.metrics(&workers, &sync_workers, &signal_refs, &trace_log);
                            info!(target: "metrics", "metrics: {:?}", metrics);
                            trace::finish_rt(
                                trace_log.as_mut(),
                                timing,
                                "Printing runtime metrics",
                                &[],
                            );
                        }
                    }
                    token if token.0 < SYNC_WORKER_ID_START => {
                        let timing = trace::start(&trace_log);
                        handle_worker_event(&mut workers, event)?;
                        trace::finish_rt(
                            trace_log.as_mut(),
                            timing,
                            "Processing worker event",
                            &[],
                        );
                    }
                    token if token.0 <= SYNC_WORKER_ID_END => {
                        let timing = trace::start(&trace_log);
                        handle_sync_worker_event(&mut sync_workers, event)?;
                        trace::finish_rt(
                            trace_log.as_mut(),
                            timing,
                            "Processing sync worker event",
                            &[],
                        );
                    }
                    _ => debug!("unexpected OS event: {:?}", event),
                }
            }
            trace::finish_rt(trace_log.as_mut(), timing, "Handling OS events", &[]);

            // Once all (sync) worker threads are done running we can return.
            if workers.is_empty() && sync_workers.is_empty() {
                return Ok(());
            }
        }
    }

    /// Gather metrics about the coordinator and runtime.
    fn metrics<'l>(
        &self,
        workers: &[Worker],
        sync_workers: &[SyncWorker],
        signal_refs: &ActorGroup<Signal>,
        trace_log: &'l Option<trace::CoordinatorLog>,
    ) -> Metrics<'l> {
        Metrics {
            worker_threads: workers.len(),
            sync_actors: sync_workers.len(),
            shared: self.internals.metrics(),
            process_signals: SIGNAL_SET,
            process_signal_receivers: signal_refs.len(),
            trace_log: trace_log.as_ref().map(trace::CoordinatorLog::metrics),
        }
    }
}

/// Set of signals we're listening for.
const SIGNAL_SET: SignalSet = SignalSet::all();

/// Setup a new `Signals` instance, registering it with `registry`.
fn setup_signals(registry: &Registry) -> io::Result<Signals> {
    trace!("setting up signal handling: signals={:?}", SIGNAL_SET);
    Signals::new(SIGNAL_SET).and_then(|mut signals| {
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

/// Relay all signals received from `signals` to the `workers` and
/// `signal_refs`.
/// Returns a bool indicating we received `SIGUSR2`, which is used to get
/// metrics from the runtime. If this returns `true` call `log_metrics`.
fn relay_signals(
    signals: &mut Signals,
    workers: &mut [Worker],
    signal_refs: &mut ActorGroup<Signal>,
) -> bool {
    signal_refs.remove_disconnected();

    let mut log_metrics = false;
    loop {
        match signals.receive() {
            Ok(Some(signal)) => {
                let signal = Signal::from_mio(signal);
                if let Signal::User2 = signal {
                    log_metrics = true;
                }

                debug!(
                    "relaying process signal to worker threads: signal={:?}",
                    signal
                );
                for worker in workers.iter_mut() {
                    if let Err(err) = worker.send_signal(signal) {
                        // NOTE: if the worker is unable to receive a message
                        // it's likely already shutdown or is shutting down.
                        // Rather than returning the error here and stopping the
                        // coordinator (which was the case previously) we log
                        // the error and instead wait until the worker thread
                        // stopped returning that error instead, which is likely
                        // more useful (i.e. it has the reason why the worker
                        // thread stopped).
                        error!(
                            "failed to send process signal to worker: {}: signal={}, worker={}",
                            err,
                            signal,
                            worker.id()
                        );
                    }
                }
                if !signal_refs.is_empty() {
                    // Safety: only returns an error if the group is empty, so
                    // this `unwrap` is safe.
                    signal_refs.try_send(signal, Delivery::ToAll).unwrap();
                }
            }
            Ok(None) => break,
            Err(err) => {
                error!("failed to retrieve process signal: {}", err);
                break;
            }
        }
    }
    log_metrics
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
            // Receiving end of the pipe is dropped, which means the sync worker
            // has shut down.
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
    /// Error polling ([`mio::Poll`]).
    Polling(io::Error),
    /// Error sending start signal to worker.
    SendingStartSignal(io::Error),
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
            Polling(err) => write!(f, "error polling for OS events: {}", err),
            SendingStartSignal(err) => write!(f, "error sending start signal to worker: {}", err),
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
            | SendingFunc(ref err) => Some(err),
        }
    }
}
