//! Coordinator thread code.

use std::env::consts::ARCH;
use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::os::unix::process::parent_id;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io, process};

use log::{debug, error, info, trace};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::actor_ref::{ActorGroup, Delivery};
use crate::rt::shared::waker;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{
    self, cpu_usage, shared, Signal, SyncWorker, Worker, SYNC_WORKER_ID_END, SYNC_WORKER_ID_START,
};
use crate::trace;

/// Token used to receive process signals.
const SIGNAL: Token = Token(usize::MAX);

#[derive(Debug)]
pub(super) struct Coordinator {
    /// OS name and version, from `uname(2)`.
    os: Box<str>,
    /// Name of the host. `nodename` field from `uname(2)`.
    host_name: Box<str>,
    host_id: Uuid,
    /// Name of the application.
    app_name: Box<str>,
    /// OS poll, used to poll the status of the (sync) worker threads and
    /// process `signals`.
    poll: Poll,
    /// Signal notifications.
    signals: Signals,
    /// Internals shared between the coordinator and workers.
    internals: Arc<shared::RuntimeInternals>,
    /// Start time, used to calculate [`Metrics`]'s uptime.
    start: Instant,
}

/// Metrics for [`Coordinator`].
#[derive(Debug)]
struct Metrics<'c, 'l> {
    heph_version: &'static str,
    os: &'c str,
    architecture: &'static str,
    host_name: &'c str,
    host_id: Uuid,
    app_name: &'c str,
    process_id: u32,
    parent_process_id: u32,
    uptime: Duration,
    worker_threads: usize,
    sync_actors: usize,
    shared: shared::Metrics,
    process_signals: SignalSet,
    process_signal_receivers: usize,
    total_cpu_time: Duration,
    cpu_time: Duration,
    trace_log: Option<trace::CoordinatorMetrics<'l>>,
}

#[derive(Copy, Clone)]
struct Uuid(u128);

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always force a length of 32.
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
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
        let poll = Poll::new()?;
        // NOTE: on Linux this MUST be created before starting the worker
        // threads.
        let signals = setup_signals(poll.registry())?;

        let setup = shared::RuntimeInternals::setup()?;
        let internals = Arc::new_cyclic(|shared_internals| {
            let waker_id = waker::init(shared_internals.clone());
            setup.complete(waker_id, worker_wakers, trace_log)
        });

        let (os, host_name) = host_info()?;
        let host_id = host_id()?;
        Ok(Coordinator {
            os,
            host_name,
            host_id,
            app_name,
            poll,
            signals,
            internals,
            start: Instant::now(),
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
    fn metrics<'c, 'l>(
        &'c self,
        workers: &[Worker],
        sync_workers: &[SyncWorker],
        signal_refs: &ActorGroup<Signal>,
        trace_log: &'l Option<trace::CoordinatorLog>,
    ) -> Metrics<'c, 'l> {
        let total_cpu_time = cpu_usage(libc::CLOCK_PROCESS_CPUTIME_ID);
        let cpu_time = cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID);
        Metrics {
            heph_version: concat!("v", env!("CARGO_PKG_VERSION")),
            os: &*self.os,
            architecture: ARCH,
            host_name: &*self.host_name,
            host_id: self.host_id,
            app_name: &*self.app_name,
            process_id: process::id(),
            parent_process_id: parent_id(),
            uptime: self.start.elapsed(),
            worker_threads: workers.len(),
            sync_actors: sync_workers.len(),
            shared: self.internals.metrics(),
            process_signals: SIGNAL_SET,
            process_signal_receivers: signal_refs.len(),
            total_cpu_time,
            cpu_time,
            trace_log: trace_log.as_ref().map(trace::CoordinatorLog::metrics),
        }
    }
}

/// Returns (OS name and version, hostname).
///
/// Uses `uname(2)`.
fn host_info() -> io::Result<(Box<str>, Box<str>)> {
    // NOTE: we could also use `std::env::consts::OS`, but this looks better.
    #[cfg(target_os = "linux")]
    const OS: &str = "GNU/Linux";
    #[cfg(target_os = "freebsd")]
    const OS: &str = "FreeBSD";
    #[cfg(target_os = "macos")]
    const OS: &str = "macOS";

    let mut uname_info: MaybeUninit<libc::utsname> = MaybeUninit::uninit();
    if unsafe { libc::uname(uname_info.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: call to `uname(2)` above ensures `uname_info` is initialised.
    let uname_info = unsafe { uname_info.assume_init() };
    let sysname = unsafe { CStr::from_ptr(&uname_info.sysname as *const _).to_string_lossy() };
    let release = unsafe { CStr::from_ptr(&uname_info.release as *const _).to_string_lossy() };
    let version = unsafe { CStr::from_ptr(&uname_info.version as *const _).to_string_lossy() };
    let nodename = unsafe { CStr::from_ptr(&uname_info.nodename as *const _).to_string_lossy() };

    let os = format!("{} ({} {} {})", OS, sysname, release, version).into_boxed_str();
    let hostname = nodename.into_owned().into_boxed_str();
    Ok((os, hostname))
}

/// Get the host id by reading `/etc/machine-id` on Linux or `/etc/hostid` on
/// FreeBSD.
#[cfg(any(target_os = "freebsd", target_os = "linux"))]
fn host_id() -> io::Result<Uuid> {
    use std::fs::File;
    use std::io::Read;

    // See <https://www.freedesktop.org/software/systemd/man/machine-id.html>.
    #[cfg(target_os = "linux")]
    const PATH: &str = "/etc/machine-id";
    // Hexadecimal, 32 characters.
    #[cfg(target_os = "linux")]
    const EXPECTED_SIZE: usize = 32;

    // No docs, but a bug tracker:
    // <https://bugs.freebsd.org/bugzilla/show_bug.cgi?id=255293>.
    #[cfg(target_os = "freebsd")]
    const PATH: &str = "/etc/hostid";
    // Hexadecimal, hypenated, 36 characters.
    #[cfg(target_os = "freebsd")]
    const EXPECTED_SIZE: usize = 36;

    let mut buf = [0; EXPECTED_SIZE];
    let mut file = File::open(PATH)?;
    let n = file.read(&mut buf).map_err(|err| {
        let msg = format!("can't open '{}': {}", PATH, err);
        io::Error::new(err.kind(), msg)
    })?;

    if n == EXPECTED_SIZE {
        #[cfg(target_os = "linux")]
        let res = from_hex(&buf[..EXPECTED_SIZE]);
        #[cfg(target_os = "freebsd")]
        let res = from_hex_hyphenated(&buf[..EXPECTED_SIZE]);

        res.map_err(|()| {
            let msg = format!("invalid `{}` format: input is not hex", PATH);
            io::Error::new(io::ErrorKind::InvalidData, msg)
        })
    } else {
        let msg = format!(
            "can't read '{}', invalid format: only read {} bytes (expected {})",
            PATH, n, EXPECTED_SIZE,
        );
        Err(io::Error::new(io::ErrorKind::InvalidData, msg))
    }
}

/// `input` should be 32 bytes long.
#[cfg(target_os = "linux")]
fn from_hex(input: &[u8]) -> Result<Uuid, ()> {
    let mut bytes = [0; 16];
    for (idx, chunk) in input.chunks_exact(2).enumerate() {
        let lower = from_hex_byte(chunk[1])?;
        let higher = from_hex_byte(chunk[0])?;
        bytes[idx] = lower | (higher << 4);
    }
    Ok(Uuid(u128::from_be_bytes(bytes)))
}

/// `input` should be 36 bytes long.
#[cfg(target_os = "freebsd")]
fn from_hex_hyphenated(input: &[u8]) -> Result<Uuid, ()> {
    let mut bytes = [0; 16];
    let mut idx = 0;

    // Groups of 8, 4, 4, 4, 12 bytes.
    let groups: [std::ops::Range<usize>; 5] = [0..8, 9..13, 14..18, 19..23, 24..36];

    for group in groups {
        let group_end = group.end;
        for chunk in input[group].chunks_exact(2) {
            let lower = from_hex_byte(chunk[1])?;
            let higher = from_hex_byte(chunk[0])?;
            bytes[idx] = lower | (higher << 4);
            idx += 1;
        }

        if let Some(b) = input.get(group_end) {
            if *b != b'-' {
                return Err(());
            }
        }
    }

    Ok(Uuid(u128::from_be_bytes(bytes)))
}

#[cfg(any(target_os = "freebsd", target_os = "linux"))]
const fn from_hex_byte(b: u8) -> Result<u8, ()> {
    match b {
        b'A'..=b'F' => Ok(b - b'A' + 10),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'0'..=b'9' => Ok(b - b'0'),
        _ => Err(()),
    }
}

/// Gets the host id by calling `gethostuuid` on macOS.
#[cfg(target_os = "macos")]
fn host_id() -> io::Result<Uuid> {
    let mut bytes = [0; 16];
    let timeout = libc::timespec {
        tv_sec: 1, // This shouldn't block, but just in case. SQLite does this also.
        tv_nsec: 0,
    };
    if unsafe { libc::gethostuuid(bytes.as_mut_ptr(), &timeout) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(Uuid(u128::from_be_bytes(bytes)))
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
