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

use std::cmp::max;
use std::env::consts::ARCH;
use std::os::unix::process::parent_id;
use std::sync::Arc;
use std::task::{self, Poll};
use std::time::Instant;
use std::{fmt, io, process};

use a10::signals::{ReceiveSignals, Signals};
use heph::actor_ref::{ActorGroup, Delivery};
use log::{as_debug, as_display, debug, error, info};

use crate::setup::{host_id, host_info, Uuid};
use crate::signal::{Signal, ALL_SIGNALS};
use crate::wakers::ring_waker;
use crate::wakers::shared::Wakers;
use crate::{self as rt, cpu_usage, shared, sync_worker, trace, worker};

/// Coordinator responsible for coordinating the Heph runtime.
pub(crate) struct Coordinator {
    /// io_uring completion ring.
    ring: a10::Ring,
    /// Internals shared between the coordinator and all (sync) workers.
    internals: Arc<shared::RuntimeInternals>,
    /// Process signal receiver.
    signals: ReceiveSignals,
    // Data used in [`Coordinator::log_metrics`].
    /// Start time of the application.
    start: Instant,
    /// Name of the application.
    app_name: Box<str>,
    /// OS name and version, from `uname(2)`.
    host_os: Box<str>,
    /// Name of the host, `nodename` field from `uname(2)`.
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
    pub(crate) fn init(
        ring: a10::Ring,
        app_name: Box<str>,
        worker_sqs: Box<[a10::SubmissionQueue]>,
        trace_log: Option<Arc<trace::SharedLog>>,
    ) -> io::Result<Coordinator> {
        let signals = ALL_SIGNALS.into_iter().map(Signal::to_signo);
        let signals = match Signals::from_signals(ring.submission_queue().clone(), signals) {
            Ok(signals) => signals.receive_signals(),
            Err(err) => {
                return Err(io::Error::new(
                    err.kind(),
                    format!("failed to setup process signal handling: {err}"),
                ))
            }
        };

        let sq = ring.submission_queue().clone();
        #[allow(clippy::cast_possible_truncation)]
        let entries = max((worker_sqs.len() * 64) as u32, 8);
        let setup = shared::RuntimeInternals::setup(sq, entries)?;
        let internals = Arc::new_cyclic(|shared_internals| {
            let wakers = Wakers::new(shared_internals.clone());
            setup.complete(wakers, worker_sqs, trace_log)
        });

        let (host_os, host_name) = host_info()?;
        let host_id = host_id()?;
        Ok(Coordinator {
            ring,
            internals,
            signals,
            start: Instant::now(),
            app_name,
            host_os,
            host_name,
            host_id,
        })
    }

    /// Get access to the shared runtime internals.
    pub(crate) const fn shared_internals(&self) -> &Arc<shared::RuntimeInternals> {
        &self.internals
    }

    /// Run the coordinator.
    ///
    /// # Notes
    ///
    /// `workers` and `sync_workers` must be sorted based on `id`.
    pub(crate) fn run(
        mut self,
        mut workers: Vec<worker::Handle>,
        mut sync_workers: Vec<sync_worker::Handle>,
        mut signal_refs: ActorGroup<Signal>,
        mut trace_log: Option<trace::CoordinatorLog>,
    ) -> Result<(), rt::Error> {
        // Signal to all worker threads the runtime was started. See
        // `local::RuntimeInternals::started` why this is needed.
        debug!("signaling to workers the runtime started");
        for worker in &workers {
            worker
                .send_runtime_started()
                .map_err(|err| rt::Error::coordinator(Error::SendingStartSignal(err)))?;
        }

        // Start listening for process signals.
        self.check_process_signals(
            &mut workers,
            &sync_workers,
            &mut signal_refs,
            &mut trace_log,
        )
        .map_err(|err| rt::Error::coordinator(Error::SignalHandling(err)))?;

        loop {
            let timing = trace::start(&trace_log);
            self.ring
                .poll(None)
                .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
            trace::finish_rt(trace_log.as_mut(), timing, "Polling for OS events", &[]);

            // We get awoken when we get a process signal.
            let timing = trace::start(&trace_log);
            self.check_process_signals(
                &mut workers,
                &sync_workers,
                &mut signal_refs,
                &mut trace_log,
            )
            .map_err(|err| rt::Error::coordinator(Error::SignalHandling(err)))?;
            trace::finish_rt(trace_log.as_mut(), timing, "Checking process signals", &[]);

            // When a (sync) worker stops it will wake to coordinator, so check
            // for a change in the worker threads.
            let timing = trace::start(&trace_log);
            check_workers(&mut workers, &mut sync_workers)?;
            trace::finish_rt(trace_log.as_mut(), timing, "Checking workers", &[]);

            // Once all (sync) workers are done running we can return.
            if workers.is_empty() && sync_workers.is_empty() {
                return Ok(());
            }
        }
    }

    /// Check if a process signal was received, relaying it to the `workers` and
    /// actors in `signal_refs`.
    fn check_process_signals(
        &mut self,
        workers: &mut [worker::Handle],
        sync_workers: &[sync_worker::Handle],
        signal_refs: &mut ActorGroup<Signal>,
        trace_log: &mut Option<trace::CoordinatorLog>,
    ) -> io::Result<()> {
        let waker = ring_waker::new(self.ring.submission_queue().clone());
        let mut ctx = task::Context::from_waker(&waker);
        loop {
            match self.signals.poll_signal(&mut ctx) {
                Poll::Ready(Some(Ok(info))) => {
                    #[allow(clippy::cast_possible_wrap)]
                    let Some(signal) = Signal::from_signo(info.ssi_signo as _) else {
                        debug!("received unexpected signal (number {})", info.ssi_signo);
                        continue;
                    };
                    if let Signal::User2 = signal {
                        self.log_metrics(workers, sync_workers, signal_refs, trace_log);
                    }

                    debug!(signal = as_debug!(signal); "relaying process signal to worker threads");
                    for worker in &mut *workers {
                        if let Err(err) = worker.send_signal(signal) {
                            // NOTE: if the worker is unable to receive a
                            // message it's likely already shutdown or is
                            // shutting down. Rather than returning the error
                            // here and stopping the coordinator (which was the
                            // case previously) we log the error and instead
                            // wait until the worker thread stopped returning
                            // that error instead, which is likely more useful
                            // (i.e. it has the reason why the worker thread
                            // stopped).
                            error!(
                                signal = as_debug!(signal), worker_id = worker.id();
                                "failed to send process signal to worker: {err}",
                            );
                        }
                    }

                    debug!(signal = as_debug!(signal); "relaying process signal to actors");
                    signal_refs.remove_disconnected();
                    _ = signal_refs.try_send(signal, Delivery::ToAll);
                }
                Poll::Ready(Some(Err(err))) => return Err(err),
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        Ok(())
    }

    /// Log metrics about the coordinator and runtime.
    fn log_metrics(
        &self,
        workers: &[worker::Handle],
        sync_workers: &[sync_worker::Handle],
        signal_refs: &ActorGroup<Signal>,
        trace_log: &mut Option<trace::CoordinatorLog>,
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
            process_signals = as_debug!(ALL_SIGNALS),
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

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Coordinator")
            .field("ring", &self.ring)
            .field("internals", &self.internals)
            .field("start", &self.start)
            .field("app_name", &self.app_name)
            .field("host_os", &self.host_os)
            .field("host_name", &self.host_name)
            .field("host_id", &self.host_id)
            .finish()
    }
}

/// Check if the (sync) workers are still alive, removing any that are not.
fn check_workers(
    workers: &mut Vec<worker::Handle>,
    sync_workers: &mut Vec<sync_worker::Handle>,
) -> Result<(), rt::Error> {
    for worker in workers.extract_if(|w| w.is_finished()) {
        debug!(worker_id = worker.id(); "worker thread stopped");
        worker
            .join()
            .map_err(rt::Error::worker_panic)
            .and_then(|res| res)?;
    }

    for sync_worker in sync_workers.extract_if(|w| w.is_finished()) {
        debug!(sync_worker_id = sync_worker.id(); "sync actor worker thread stopped");
        sync_worker.join().map_err(rt::Error::sync_actor_panic)?;
    }

    Ok(())
}

/// Error running the [`Coordinator`].
#[derive(Debug)]
pub(crate) enum Error {
    /// Error polling [`a10::Ring`].
    Polling(io::Error),
    /// Error handling a process signal.
    SignalHandling(io::Error),
    /// Error sending start signal to worker.
    SendingStartSignal(io::Error),
    /// Error sending function to worker.
    SendingFunc(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Polling(err) => write!(f, "error polling for OS events: {err}"),
            Error::SignalHandling(err) => write!(f, "error handling process signal: {err}"),
            Error::SendingStartSignal(err) => {
                write!(f, "error sending start signal to worker: {err}")
            }
            Error::SendingFunc(err) => write!(f, "error sending function to worker: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Polling(ref err)
            | Error::SignalHandling(ref err)
            | Error::SendingStartSignal(ref err)
            | Error::SendingFunc(ref err) => Some(err),
        }
    }
}
