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
use std::time::{Duration, Instant};
use std::{fmt, io, process};

use a10::process::{ReceiveSignals, Signals};
use heph::actor_ref::ActorGroup;
use log::{debug, error, info, trace};

use crate::setup::{host_id, host_info, Uuid};
use crate::{self as rt, cpu_usage, shared, sync_worker, trace, worker, Signal};

/// Setup the [`Coordinator`].
pub(crate) fn setup(app_name: Box<str>, threads: usize) -> Result<CoordinatorSetup, rt::Error> {
    let (host_os, host_name) = host_info().map_err(rt::Error::init_coordinator)?;
    let host_id = host_id().map_err(rt::Error::init_coordinator)?;

    // At most we expect each worker thread to generate a single completion and
    // a possibly an incoming signal.
    #[allow(clippy::cast_possible_truncation)]
    let entries = max((threads + 1).next_power_of_two() as u32, 8);
    let ring = a10::Ring::config(entries)
        .single_issuer()
        .with_kernel_thread(true)
        .build()
        .map_err(rt::Error::init_coordinator)?;
    let sq = ring.submission_queue();

    // NOTE: signal handling MUST be setup before spawning the worker threads as
    // they need to inherint the signal handling properties.
    let signals = Signal::ALL.into_iter().map(Signal::to_signo);
    let signals = match Signals::from_signals(sq.clone(), signals) {
        Ok(signals) => signals.receive_signals(),
        Err(err) => {
            return Err(rt::Error::init_coordinator(io::Error::new(
                err.kind(),
                format!("failed to setup process signal handling: {err}"),
            )))
        }
    };

    Ok(CoordinatorSetup {
        ring,
        signals,
        app_name,
        host_os,
        host_name,
        host_id,
    })
}

/// Setup the [`Coordinator`].
///
/// # Notes
///
/// This must be called before creating the worker threads to properly catch
/// process signals.
#[derive(Debug)]
pub(crate) struct CoordinatorSetup {
    ring: a10::Ring,
    signals: ReceiveSignals,
    app_name: Box<str>,
    host_os: Box<str>,
    host_name: Box<str>,
    host_id: Uuid,
}

impl CoordinatorSetup {
    /// Coordinator's submission queue.
    pub(crate) fn submission_queue(&self) -> &a10::SubmissionQueue {
        self.ring.submission_queue()
    }

    /// Complete the coordinator setup.
    pub(crate) fn complete(
        self,
        internals: Arc<shared::RuntimeInternals>,
        workers: Vec<worker::Handle>,
        sync_workers: Vec<sync_worker::Handle>,
        signal_refs: ActorGroup<Signal>,
        trace_log: Option<trace::CoordinatorLog>,
    ) -> Coordinator {
        Coordinator {
            ring: self.ring,
            internals,
            workers,
            sync_workers,
            signals: self.signals,
            signal_refs,
            trace_log,
            start: Instant::now(),
            app_name: self.app_name,
            host_os: self.host_os,
            host_name: self.host_name,
            host_id: self.host_id,
        }
    }
}

/// Coordinator responsible for coordinating the Heph runtime.
pub(crate) struct Coordinator {
    /// io_uring completion ring.
    ring: a10::Ring,
    /// Internals shared between the coordinator and all (sync) workers.
    internals: Arc<shared::RuntimeInternals>,
    /// Handles to the worker threads.
    workers: Vec<worker::Handle>,
    /// Handles to the sync worker threads.
    sync_workers: Vec<sync_worker::Handle>,
    /// Process signal receiver.
    signals: ReceiveSignals,
    /// Actor that want to receive a process signal.
    signal_refs: ActorGroup<Signal>,
    /// Trace log for the coordinator.
    trace_log: Option<trace::CoordinatorLog>,
    // Data used in [`Coordinator::log_metrics`].
    /// Time when the coordinator start running.
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
    /// Run the coordinator.
    ///
    /// # Notes
    ///
    /// `workers` and `sync_workers` must be sorted based on `id`.
    pub(crate) fn run(mut self) -> Result<(), rt::Error> {
        self.start = Instant::now();
        // Signal to all worker threads the runtime was started. See
        // `local::RuntimeInternals::started` why this is needed.
        debug!("signaling to workers the runtime started");
        for worker in &self.workers {
            worker
                .send_runtime_started()
                .map_err(|err| rt::Error::coordinator(Error::SendingStartSignal(err)))?;
        }

        // Start listening for process signals.
        self.check_process_signals(&mut false)?;

        let mut timeout = None;
        loop {
            // Wait for something to happen.
            self.poll_os(timeout)?;
            // Normally we would get informed what "thing" happened, but to keep
            // the coordinator simple we don't. We simply wake the coordinator
            // and check everything.

            // There is a race between 1) the waking of the coordinator by the
            // (sync) workers when they are dropped (the call to
            // `shared::RuntimeInternals::wake_coordinator` in their `Drop`
            // implementation) and 2) the coordinator checking if the (sync)
            // workers are finished.
            // If the time between 1) and 2) is too short then the thread might
            // not yet be terminated. To prevent the coordinator from waiting
            // for ever on nothing (because all worker threads have finished) we
            // poll in next loop with a small-ish timeout (see `timeout`
            // below`).
            let mut wake_up_reason_found = false;

            // First check for process signals.
            self.check_process_signals(&mut wake_up_reason_found)?;

            // Next check the (sync) workers.
            self.check_workers(&mut wake_up_reason_found)?;

            // Once all (sync) workers are done running we can return.
            if self.workers.is_empty() && self.sync_workers.is_empty() {
                return Ok(());
            }

            timeout = (!wake_up_reason_found).then(|| Duration::from_millis(100));
        }
    }

    /// Poll the [`a10::Ring`].
    fn poll_os(&mut self, timeout: Option<Duration>) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        self.ring
            .poll(timeout)
            .map_err(|err| rt::Error::coordinator(Error::Polling(err)))?;
        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Polling for OS events",
            &[],
        );
        Ok(())
    }

    /// Check if a process signal was received, relaying it to the `workers` and
    /// actors in `signal_refs`.
    fn check_process_signals(&mut self, signal_received: &mut bool) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        let waker = task::Waker::noop();
        let mut ctx = task::Context::from_waker(waker);
        loop {
            match self.signals.poll_signal(&mut ctx) {
                Poll::Ready(Some(Ok(info))) => {
                    *signal_received = true;
                    #[allow(clippy::cast_possible_wrap)]
                    let Some(signal) = Signal::from_signo(info.ssi_signo as _) else {
                        debug!(signal_number = info.ssi_signo, signal_code = info.ssi_code,
                            sending_pid = info.ssi_pid, sending_uid = info.ssi_uid; "received unexpected signal, not relaying");
                        continue;
                    };
                    debug!(signal:? = signal, signal_number = info.ssi_signo, signal_code = info.ssi_code,
                        sending_pid = info.ssi_pid, sending_uid = info.ssi_uid; "received process signal");

                    if let Signal::User2 = signal {
                        self.log_metrics();
                    }

                    trace!(signal:? = signal; "relaying process signal to worker threads");
                    for worker in &mut self.workers {
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
                                signal:? = signal, worker_id = worker.id();
                                "failed to send process signal to worker: {err}",
                            );
                        }
                    }

                    trace!(signal:? = signal; "relaying process signal to actors");
                    self.signal_refs.remove_disconnected();
                    _ = self.signal_refs.try_send_to_all(signal);
                }
                Poll::Ready(Some(Err(err))) => {
                    return Err(rt::Error::coordinator(Error::SignalHandling(err)))
                }
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Checking process signals",
            &[],
        );
        Ok(())
    }

    /// Log metrics about the coordinator and runtime.
    fn log_metrics(&mut self) {
        let timing = trace::start(&self.trace_log);
        let shared_metrics = self.internals.metrics();
        let trace_metrics = self.trace_log.as_ref().map(trace::CoordinatorLog::metrics);
        info!(
            target: "metrics",
            heph_version = concat!("v", env!("CARGO_PKG_VERSION")),
            host_os = self.host_os,
            host_arch = ARCH,
            host_name = self.host_name,
            host_id:% = self.host_id,
            app_name = self.app_name,
            process_id = process::id(),
            parent_process_id = parent_id(),
            uptime:? = self.start.elapsed(),
            worker_threads = self.workers.len(),
            sync_actors = self.sync_workers.len(),
            shared_scheduler_ready = shared_metrics.scheduler_ready,
            shared_scheduler_inactive = shared_metrics.scheduler_inactive,
            shared_timers_total = shared_metrics.timers_total,
            shared_timers_next:? = shared_metrics.timers_next,
            process_signals:? = Signal::ALL,
            process_signal_receivers = self.signal_refs.len(),
            cpu_time:? = cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID),
            total_cpu_time:? = cpu_usage(libc::CLOCK_PROCESS_CPUTIME_ID),
            trace_file:? = trace_metrics.as_ref().map(|m| m.file),
            trace_counter = trace_metrics.map_or(0, |m| m.counter);
            "coordinator metrics",
        );
        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Printing runtime metrics",
            &[],
        );
    }

    /// Check if the (sync) workers are still alive, removing any that are not.
    fn check_workers(&mut self, worker_stopped: &mut bool) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        for worker in self.workers.extract_if(|w| w.is_finished()) {
            *worker_stopped = true;
            debug!(worker_id = worker.id(); "worker thread stopped");
            worker
                .join()
                .map_err(rt::Error::worker_panic)
                .and_then(|res| res)?;
        }

        for sync_worker in self.sync_workers.extract_if(|w| w.is_finished()) {
            *worker_stopped = true;
            debug!(sync_worker_id = sync_worker.id(); "sync actor worker thread stopped");
            sync_worker.join().map_err(rt::Error::sync_actor_panic)?;
        }

        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Checking (sync) workers",
            &[],
        );
        Ok(())
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
