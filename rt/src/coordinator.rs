//! Coordinator thread code.
//!
//! The coordinator, as the name suggests, coordinates the Heph runtime. This
//! includes monitoring the [worker threads] running the thread-safe and
//! thread-local actors and futures, monitoring the [sync worker threads]
//! running the synchronous actors and handling process signals.
//!
//! Most of the time the coordinator is polling new events to handle, i.e.
//! waiting for something to happen. Such an event can be one of the following:
//! * An incoming process signal, which it relays to all registers actors and
//!   (sync) worker threads.
//! * A (sync) worker thread stopping because all actors have finished running,
//!   the worker hit an error or the thread panicked.
//!
//! [worker threads]: crate::worker
//! [sync worker threads]: crate::sync_worker

use std::env::consts::ARCH;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::parent_id;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io, process};

use heph::actor_ref::{ActorGroup, Delivery};
use log::{as_debug, as_display, debug, error, info, trace};
use mio::event::Event;
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::setup::{host_id, host_info, Uuid};
use crate::shared::waker;
use crate::thread_waker::ThreadWaker;
use crate::trace;
use crate::{
    self as rt, cpu_usage, shared, worker, Signal, SyncWorker, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};

/// Token used to receive process signals.
const SIGNAL: Token = Token(usize::MAX);
const RING: Token = Token(usize::MAX - 1);

/// Coordinator responsible for coordinating the Heph runtime.
#[derive(Debug)]
pub(super) struct Coordinator {
    /// I/O uring.
    ring: a10::Ring,
    /// OS poll, used to poll the status of the (sync) worker threads and
    /// process `signals`.
    poll: Poll,
    /// Process signal notifications.
    signals: Signals,
    /// Internals shared between the coordinator and all workers.
    internals: Arc<shared::RuntimeInternals>,

    // Data used in [`Metrics`].
    /// Start time, used to calculate [`Metrics`]'s uptime.
    start: Instant,
    /// Name of the application.
    app_name: Box<str>,
    /// OS name and version, from `uname(2)`.
    host_os: Box<str>,
    /// Name of the host. `nodename` field from `uname(2)`.
    host_name: Box<str>,
    /// Id of the host.
    host_id: Uuid,
}

impl Coordinator {
    /// Initialise the `Coordinator`.
    ///
    /// # Notes
    ///
    /// This must be called before creating the worker threads to properly catch
    /// process signals.
    pub(super) fn init(
        app_name: Box<str>,
        worker_wakers: Box<[&'static ThreadWaker]>,
        trace_log: Option<Arc<trace::SharedLog>>,
    ) -> io::Result<Coordinator> {
        let ring = a10::Ring::config(512).build()?;
        let poll = Poll::new()?;
        // NOTE: on Linux this MUST be created before starting the worker
        // threads.
        let signals = setup_signals(poll.registry())?;

        let setup = shared::RuntimeInternals::setup()?;
        let internals = Arc::new_cyclic(|shared_internals| {
            let waker_id = waker::init(shared_internals.clone());
            setup.complete(waker_id, worker_wakers, trace_log)
        });

        let (host_os, host_name) = host_info()?;
        let host_id = host_id()?;
        Ok(Coordinator {
            ring,
            poll,
            signals,
            internals,
            start: Instant::now(),
            app_name,
            host_os,
            host_name,
            host_id,
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
        mut workers: Vec<worker::Handle>,
        mut sync_workers: Vec<SyncWorker>,
        mut signal_refs: ActorGroup<Signal>,
        mut trace_log: Option<trace::CoordinatorLog>,
    ) -> Result<(), rt::Error> {
        self.pre_run(&mut workers, &mut sync_workers, &mut trace_log)?;

        let mut events = Events::with_capacity(16);
        loop {
            let timing = trace::start(&trace_log);
            self.poll_os(&mut events)
                .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
            trace::finish_rt(trace_log.as_mut(), timing, "Polling for OS events", &[]);

            let timing = trace::start(&trace_log);
            for event in events.iter() {
                trace!("got OS event: {event:?}");

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
                            self.log_metrics(&workers, &sync_workers, &signal_refs, &mut trace_log);
                        }
                    }
                    RING => {
                        self.ring
                            .poll(Some(Duration::ZERO))
                            .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
                        self.poll
                            .registry()
                            .reregister(
                                &mut SourceFd(&self.ring.as_fd().as_raw_fd()),
                                RING,
                                Interest::READABLE,
                            )
                            .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
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
                    _ => debug!("unexpected OS event: {event:?}"),
                }
            }
            trace::finish_rt(trace_log.as_mut(), timing, "Handling OS events", &[]);

            // Once all (sync) worker threads are done running we can return.
            if workers.is_empty() && sync_workers.is_empty() {
                return Ok(());
            }
        }
    }

    fn poll_os(&mut self, events: &mut Events) -> io::Result<()> {
        match self.poll.poll(events, None) {
            Ok(()) => Ok(()),
            // The I/O uring will interrupt us.
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Do the pre-[`run`] setup.
    ///
    /// [`run`]: Coordinator::run
    fn pre_run(
        &mut self,
        workers: &mut [worker::Handle],
        sync_workers: &mut Vec<SyncWorker>,
        trace_log: &mut Option<trace::CoordinatorLog>,
    ) -> Result<(), rt::Error> {
        debug_assert!(workers.is_sorted_by_key(worker::Handle::id));
        debug_assert!(sync_workers.is_sorted_by_key(SyncWorker::id));

        // Register various sources of OS events that need to wake us from
        // polling events.
        let timing = trace::start(&*trace_log);
        let registry = self.poll.registry();
        let ring_fd = &self.ring.as_fd().as_raw_fd();
        registry
            .register(&mut SourceFd(ring_fd), RING, Interest::READABLE)
            .map_err(|err| rt::Error::coordinator(Error::Startup(err)))?;
        register_workers(registry, workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringWorkers(err)))?;
        register_sync_workers(registry, sync_workers)
            .map_err(|err| rt::Error::coordinator(Error::RegisteringSyncActors(err)))?;
        // It could be that before we're able to register the sync workers above
        // they've already completed. On macOS and Linux this will still create
        // an kqueue/epoll event, even if the other side of the Unix pipe was
        // already disconnected before registering it with `Poll`.
        // However this doesn't seem to be the case on FreeBSD, so we explicitly
        // check if the sync workers are still alive *after* we registered them.
        check_sync_worker_alive(sync_workers)?;
        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Initialising the coordinator thread",
            &[],
        );

        // Signal to all worker threads the runtime was started. See
        // `local::Runtime.started` why this is needed.
        debug!("signaling to workers the runtime started");
        for worker in workers {
            worker
                .send_runtime_started()
                .map_err(|err| rt::Error::coordinator(Error::SendingStartSignal(err)))?;
        }
        Ok(())
    }

    /// Log metrics about the coordinator and runtime.
    fn log_metrics<'c, 'l>(
        &'c self,
        workers: &[worker::Handle],
        sync_workers: &[SyncWorker],
        signal_refs: &ActorGroup<Signal>,
        trace_log: &'l mut Option<trace::CoordinatorLog>,
    ) {
        let timing = trace::start(trace_log);
        let shared_metrics = self.internals.metrics();
        let trace_metrics = trace_log.as_ref().map(trace::CoordinatorLog::metrics);
        info!(
            target: "metrics",
            heph_version = as_display!(concat!("v", env!("CARGO_PKG_VERSION"))),
            host_os = self.host_os,
            host_arch = ARCH,
            host_name = self.host_name,
            host_id = as_display!(self.host_id),
            app_name = self.app_name,
            process_id = process::id(),
            parent_process_id = parent_id(),
            uptime = as_debug!(self.start.elapsed()),
            worker_threads = workers.len(),
            sync_actors = sync_workers.len(),
            shared_scheduler_ready = shared_metrics.scheduler_ready,
            shared_scheduler_inactive = shared_metrics.scheduler_inactive,
            shared_timers_total = shared_metrics.timers_total,
            shared_timers_next = as_debug!(shared_metrics.timers_next),
            process_signals = as_debug!(SIGNAL_SET),
            process_signal_receivers = signal_refs.len(),
            cpu_time = as_debug!(cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID)),
            total_cpu_time = as_debug!(cpu_usage(libc::CLOCK_PROCESS_CPUTIME_ID)),
            trace_file = as_debug!(trace_metrics.as_ref().map(|m| m.file)),
            trace_counter = trace_metrics.map_or(0, |m| m.counter);
            "coordinator metrics",
        );
        trace::finish_rt(trace_log.as_mut(), timing, "Printing runtime metrics", &[]);
    }
}

/// Set of signals we're listening for.
const SIGNAL_SET: SignalSet = SignalSet::all();

/// Setup a new `Signals` instance, registering it with `registry`.
fn setup_signals(registry: &Registry) -> io::Result<Signals> {
    trace!(signals = as_debug!(SIGNAL_SET); "setting up signal handling");
    Signals::new(SIGNAL_SET).and_then(|mut signals| {
        registry
            .register(&mut signals, SIGNAL, Interest::READABLE)
            .map(|()| signals)
    })
}

/// Register all `workers`' sending end of the pipe with `registry`.
fn register_workers(registry: &Registry, workers: &mut [worker::Handle]) -> io::Result<()> {
    workers.iter_mut().try_for_each(|worker| {
        trace!(worker_id = worker.id(); "registering worker thread");
        worker.register(registry)
    })
}

/// Register all `sync_workers`' sending end of the pipe with `registry`.
fn register_sync_workers(registry: &Registry, sync_workers: &mut [SyncWorker]) -> io::Result<()> {
    sync_workers.iter_mut().try_for_each(|sync_worker| {
        trace!(sync_worker_id = sync_worker.id(); "registering sync actor worker thread");
        sync_worker.register(registry)
    })
}

/// Checks all `sync_workers` if they're alive and removes any that have
/// stopped.
fn check_sync_worker_alive(sync_workers: &mut Vec<SyncWorker>) -> Result<(), rt::Error> {
    sync_workers
        .drain_filter(|sync_worker| !sync_worker.is_alive())
        .try_for_each(|sync_worker| {
            debug!(sync_worker_id = sync_worker.id(); "sync actor worker thread stopped");
            sync_worker.join().map_err(rt::Error::sync_actor_panic)
        })
}

/// Relay all signals received from `signals` to the `workers` and
/// `signal_refs`.
/// Returns a bool indicating we received `SIGUSR2`, which is used to get
/// metrics from the runtime. If this returns `true` call `log_metrics`.
fn relay_signals(
    signals: &mut Signals,
    workers: &mut [worker::Handle],
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

                debug!(signal = as_debug!(signal); "relaying process signal to worker threads");
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
                            signal = as_debug!(signal), worker_id = worker.id();
                            "failed to send process signal to worker: {err}",
                        );
                    }
                }

                debug!(signal = as_debug!(signal); "relaying process signal to actors");
                _ = signal_refs.try_send(signal, Delivery::ToAll);
            }
            Ok(None) => break,
            Err(err) => {
                error!("failed to retrieve process signal: {err}");
                break;
            }
        }
    }
    log_metrics
}

/// Handle an `event` for a worker.
fn handle_worker_event(workers: &mut Vec<worker::Handle>, event: &Event) -> Result<(), rt::Error> {
    if let Ok(i) = workers.binary_search_by_key(&event.token().0, worker::Handle::id) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the worker has
            // shut down.
            let worker = workers.remove(i);
            debug!(worker_id = worker.id(); "worker thread stopped");
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
            debug!(sync_worker_id = sync_worker.id(); "sync actor worker thread stopped");
            sync_worker.join().map_err(rt::Error::sync_actor_panic)?;
        }
    }
    Ok(())
}

/// Error running the [`Coordinator`].
#[derive(Debug)]
pub(super) enum Error {
    /// Error in starting up the Coordinator.
    Startup(io::Error),
    /// Error in [`register_workers`].
    RegisteringWorkers(io::Error),
    /// Error in [`register_sync_workers`].
    RegisteringSyncActors(io::Error),
    /// Error polling ([`mio::Poll`] or [`a10::Ring`]).
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
            Startup(err) => write!(f, "error starting coordinator: {err}"),
            RegisteringWorkers(err) => write!(f, "error registering worker threads: {err}"),
            RegisteringSyncActors(err) => {
                write!(f, "error registering synchronous actor threads: {err}")
            }
            Polling(err) => write!(f, "error polling for OS events: {err}"),
            SendingStartSignal(err) => write!(f, "error sending start signal to worker: {err}"),
            SendingFunc(err) => write!(f, "error sending function to worker: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            Startup(ref err)
            | RegisteringWorkers(ref err)
            | RegisteringSyncActors(ref err)
            | Polling(ref err)
            | SendingStartSignal(ref err)
            | SendingFunc(ref err) => Some(err),
        }
    }
}
