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
use std::os::unix::process::parent_id;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, io};

use a10::process::{ReceiveSignals, Signals};
use heph::actor_ref::{ActorGroup, SendError};

use crate::bitmap::{self, AtomicBitMap};
use crate::host::{self, Uuid};
use crate::{self as rt, cpu_usage, process, shared, sync_worker, trace, worker};

/// Setup the [`Coordinator`].
///
/// # Notes
///
/// This must be called before creating the worker threads to properly catch
/// process signals.
pub(crate) fn setup(app_name: Box<str>) -> io::Result<Setup> {
    let config = a10::Ring::config();
    #[cfg(any(target_os = "android", target_os = "linux"))]
    let config = config.single_issuer().defer_task_run();
    let ring = config.build()?;

    // NOTE: signal handling MUST be setup before spawning the worker threads as
    // they need to inherint the signal handling properties.
    let signals = match Signals::for_all_signals(ring.sq()) {
        Ok(signals) => signals.receive_signals(),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!("failed to setup process signal handling: {err}"),
            ));
        }
    };

    let (host_os, host_name) = host::info()?;
    let host_id = host::id()?;

    Ok(Setup {
        ring,
        signals,
        app_name,
        host_os,
        host_name,
        host_id,
    })
}

/// Setup of a [`Coordinator`].
#[derive(Debug)]
pub(crate) struct Setup {
    ring: a10::Ring,
    signals: ReceiveSignals,
    app_name: Box<str>,
    host_os: Box<str>,
    host_name: Box<str>,
    host_id: Uuid,
}

impl Setup {
    /// Coordinator's submission queue.
    pub(crate) fn sq(&self) -> a10::SubmissionQueue {
        self.ring.sq()
    }

    /// Complete the coordinator setup.
    ///
    /// # Notes
    ///
    /// `workers` and `sync_workers` must be sorted based on `id`.
    pub(crate) fn complete(
        self,
        internals: Arc<shared::RuntimeInternals>,
        workers: Vec<worker::Handle>,
        sync_workers: Vec<sync_worker::Handle>,
        signal_refs: ActorGroup<process::Signal>,
        trace_log: Option<trace::CoordinatorLog>,
    ) -> Coordinator {
        let threads_left = workers.len() + sync_workers.len();
        let workers = workers.into_iter().map(Some).collect();
        let sync_workers = sync_workers.into_iter().map(Some).collect();

        // Bitmap used to determine what (sync) worker or process signals status
        // to check.
        let check = AtomicBitMap::new(threads_left + 1 /* signals. */);
        internals.set_shutdown_bitmap(check.clone());

        Coordinator {
            ring: self.ring,
            internals,
            workers,
            sync_workers,
            threads_left,
            signals: self.signals,
            signal_refs,
            check,
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
#[derive(Debug)]
pub(crate) struct Coordinator {
    /// io_uring completion ring.
    ring: a10::Ring,
    /// Internals shared between the coordinator and all (sync) workers.
    internals: Arc<shared::RuntimeInternals>,
    /// Handles to the worker threads.
    workers: Vec<Option<worker::Handle>>,
    /// Handles to the sync worker threads.
    sync_workers: Vec<Option<sync_worker::Handle>>,
    /// Number of threads that are still running in workers and sync_workers.
    threads_left: usize,
    /// Process signal receiver.
    signals: ReceiveSignals,
    /// Actor that want to receive a process signal.
    signal_refs: ActorGroup<process::Signal>,
    /// Bitmap to indicate what to check:
    /// 0                 => check process signals.
    /// n < workers.len() => check workers.
    /// otherwise         => check sync workers.
    check: Arc<AtomicBitMap>,
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
    pub(crate) fn run(mut self) -> Result<(), rt::Error> {
        self.start = Instant::now();
        // Signal to all worker threads the runtime was started. See
        // `local::RuntimeInternals::started` why this is needed.
        log::debug!("signaling to workers the runtime started");
        for worker in &self.workers {
            let Some(worker) = worker else {
                continue;
            };
            worker
                .send_runtime_started()
                .map_err(|SendError| rt::Error::coordinator(Error::SendingStartSignal))?;
        }

        // Start listening for process signals.
        self.relay_process_signals()?;

        loop {
            while let Some(n) = self.check.next_set() {
                match n {
                    0 => self.relay_process_signals()?,
                    n if n <= self.workers.len() => self.join_worker(n)?,
                    n => self.join_sync_worker(n)?,
                }
            }

            // Once all (sync) workers are done running we can return.
            if self.threads_left == 0 {
                return Ok(());
            }

            // Wait for something to happen.
            self.poll_os(None)?;
        }
    }

    /// Poll the [`a10::Ring`].
    fn poll_os(&mut self, timeout: Option<Duration>) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        log::trace!("coordinator polling");
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
    fn relay_process_signals(&mut self) -> Result<(), rt::Error> {
        log::trace!("checking for process signals");
        let timing = trace::start(&self.trace_log);
        let waker = bitmap::new_waker_set_bit0(self.check.clone());
        let mut ctx = task::Context::from_waker(&waker);
        loop {
            match Pin::new(&mut self.signals).poll_next(&mut ctx) {
                Poll::Ready(Some(Ok(info))) => {
                    let signal = info.signal();
                    #[cfg(any(target_os = "android", target_os = "linux"))]
                    log::debug!(signal:?, sending_pid = info.pid(), sending_uid = info.real_user_id(); "received process signal");
                    #[cfg(not(any(target_os = "android", target_os = "linux")))]
                    log::debug!(signal:?; "received process signal");
                    if let process::Signal::USER2 = signal {
                        self.log_metrics();
                    }

                    log::trace!(signal:?; "relaying process signal to worker threads");
                    for worker in self.workers.iter_mut() {
                        let Some(worker) = worker else {
                            continue;
                        };
                        if let Err(SendError) = worker.send_signal(signal) {
                            // NOTE: if the worker is unable to receive a
                            // message it's likely already shutdown or is
                            // shutting down. Rather than returning the error
                            // here and stopping the coordinator (which was the
                            // case previously) we log the error and instead
                            // wait until the worker thread stopped returning
                            // that error instead, which is likely more useful
                            // (i.e. it has the reason why the worker thread
                            // stopped).
                            log::warn!(
                                signal:?, worker_id = worker.id();
                                "failed to send process signal to worker",
                            );
                        }
                    }

                    log::trace!(signal:?; "relaying process signal to actors");
                    self.signal_refs.remove_disconnected();
                    _ = self.signal_refs.try_send_to_all(signal);
                }
                Poll::Ready(Some(Err(err))) => {
                    return Err(rt::Error::coordinator(Error::SignalHandling(err)));
                }
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Relaying process signals",
            &[],
        );
        Ok(())
    }

    /// Log metrics about the coordinator and runtime.
    fn log_metrics(&mut self) {
        let timing = trace::start(&self.trace_log);
        let shared_metrics = self.internals.metrics();
        let trace_metrics = self.trace_log.as_ref().map(trace::CoordinatorLog::metrics);
        log::info!(
            target: "metrics",
            heph_version = concat!("v", env!("CARGO_PKG_VERSION")),
            host_os = self.host_os,
            host_arch = ARCH,
            host_name = self.host_name,
            host_id:% = self.host_id,
            app_name = self.app_name,
            process_id = std::process::id(),
            parent_process_id = parent_id(),
            uptime:? = self.start.elapsed(),
            worker_threads = self.workers.len(),
            sync_actors = self.sync_workers.len(),
            shared_scheduler_ready = shared_metrics.scheduler_ready,
            shared_scheduler_inactive = shared_metrics.scheduler_inactive,
            shared_timers_total = shared_metrics.timers_total,
            shared_timers_next:? = shared_metrics.timers_next,
            process_signals:? = self.signals.set(),
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
            "Logging runtime metrics",
            &[],
        );
    }

    /// Wait on worker with id to stop.
    fn join_worker(&mut self, id: usize) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        if let Some(worker) = self.workers.get_mut(id - 1).and_then(|w| w.take()) {
            log::trace!(worker_id = worker.id(); "worker thread stopped, joining it");
            self.threads_left -= 1;
            worker.join()?;
        }
        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Joining worker",
            &[("id", &id)],
        );
        Ok(())
    }

    /// Wait on sync worker with id to stop.
    fn join_sync_worker(&mut self, id: usize) -> Result<(), rt::Error> {
        let timing = trace::start(&self.trace_log);
        if let Some(sync_worker) = self.sync_workers.get_mut(id - 1).and_then(|w| w.take()) {
            log::trace!(sync_worker_id = sync_worker.id(); "sync worker thread stopped, joining it");
            self.threads_left -= 1;
            sync_worker.join()?;
        }
        trace::finish_rt(
            self.trace_log.as_mut(),
            timing,
            "Joining sync worker",
            &[("id", &id)],
        );
        Ok(())
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
    SendingStartSignal,
    /// Error sending function to worker.
    SendingFunc,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Polling(err) => write!(f, "error polling for OS events: {err}"),
            Error::SignalHandling(err) => write!(f, "error handling process signal: {err}"),
            Error::SendingStartSignal => f.write_str("error sending start signal to worker"),
            Error::SendingFunc => f.write_str("error sending function to worker"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Polling(err) | Error::SignalHandling(err) => Some(err),
            Error::SendingStartSignal | Error::SendingFunc => None,
        }
    }
}
