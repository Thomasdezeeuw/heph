//! Worker thread code.

#[cfg(target_os = "linux")]
use std::mem;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{fmt, io, thread};

use crossbeam_channel::{self, Receiver};
#[cfg(target_os = "linux")]
use log::{debug, warn};
use mio::{Poll, Registry, Token};

use crate::rt::error::StringError;
use crate::rt::local::{Runtime, WAKER};
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::waker::WakerId;
use crate::rt::{self, shared, ProcessId, RuntimeRef, Signal};
use crate::trace;

pub(crate) struct WorkerSetup {
    id: NonZeroUsize,
    poll: Poll,
    /// Waker id used to create a `Waker` for thread-local actors.
    waker_id: WakerId,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
}

/// Setup a new worker thread.
///
/// Use [`WorkerSetup::start`] to spawn the worker thread.
pub(crate) fn setup(id: NonZeroUsize) -> io::Result<(WorkerSetup, &'static ThreadWaker)> {
    let poll = Poll::new()?;

    // Setup the waking mechanism.
    let (waker_sender, waker_events) = crossbeam_channel::unbounded();
    let waker = mio::Waker::new(poll.registry(), WAKER)?;
    let waker_id = rt::waker::init(waker, waker_sender);
    let thread_waker = rt::waker::get_thread_waker(waker_id);

    let setup = WorkerSetup {
        id,
        poll,
        waker_id,
        waker_events,
    };
    Ok((setup, thread_waker))
}

/// Handle to a worker thread.
#[derive(Debug)]
pub(super) struct Worker {
    /// Unique id (among all threads in the `Runtime`).
    id: NonZeroUsize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), rt::Error>>,
    /// Two-way communication channel to share messages with the worker thread.
    channel: rt::channel::Handle<CoordinatorMessage, WorkerMessage>,
}

/// Message send by the coordinator thread.
#[allow(variant_size_differences)] // Can't make `Run` smaller.
pub(crate) enum CoordinatorMessage {
    /// Runtime has started, i.e. [`Runtime::start`] was called.
    ///
    /// [`Runtime::start`]: rt::Runtime::start
    Started,
    /// Process received a signal.
    Signal(Signal),
    /// Run a function on the worker thread.
    Run(Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>),
}

/// Message send by the worker thread.
#[derive(Debug)]
pub(crate) enum WorkerMessage {}

impl WorkerSetup {
    /// Start a new worker thread.
    pub(super) fn start(
        self,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> io::Result<Worker> {
        rt::channel::new().and_then(|(channel, worker_handle)| {
            // Copy id to move into `Worker`, `self` moves into the spawned
            // thread.
            let id = self.id;
            thread::Builder::new()
                .name(format!("Worker {}", id))
                .spawn(move || {
                    main(
                        self,
                        worker_handle,
                        shared_internals,
                        auto_cpu_affinity,
                        trace_log,
                    )
                })
                .map(|handle| Worker {
                    id,
                    handle,
                    channel,
                })
        })
    }

    /// Return the worker's id.
    pub(super) fn id(&self) -> usize {
        self.id.get()
    }
}

impl Worker {
    /// Return the worker's id.
    pub(super) fn id(&self) -> usize {
        self.id.get()
    }

    /// Registers the channel used to communicate with the thread. Uses the
    /// [`id`] as [`Token`].
    ///
    /// [`id`]: Worker::id
    pub(super) fn register(&mut self, registry: &Registry) -> io::Result<()> {
        self.channel.register(registry, Token(self.id()))
    }

    /// Send the worker thread a signal that the runtime has started.
    pub(super) fn send_runtime_started(&mut self) -> io::Result<()> {
        let msg = CoordinatorMessage::Started;
        self.channel.try_send(msg)
    }

    /// Send the worker thread a `signal`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        let msg = CoordinatorMessage::Signal(signal);
        self.channel.try_send(msg)
    }

    /// Send the worker thread the function `f` to run.
    pub(super) fn send_function(
        &mut self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> io::Result<()> {
        let msg = CoordinatorMessage::Run(f);
        self.channel.try_send(msg)
    }

    /// Handle all incoming messages.
    pub(super) fn handle_messages(&mut self) -> io::Result<()> {
        while let Some(msg) = self.channel.try_recv()? {
            match msg {}
        }
        Ok(())
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<Result<(), rt::Error>> {
        self.handle.join()
    }
}

/// Run a worker thread, with an optional `setup` function.
fn main(
    setup: WorkerSetup,
    receiver: rt::channel::Handle<WorkerMessage, CoordinatorMessage>,
    shared_internals: Arc<shared::RuntimeInternals>,
    auto_cpu_affinity: bool,
    mut trace_log: Option<trace::Log>,
) -> Result<(), rt::Error> {
    let timing = trace::start(&trace_log);

    #[cfg(target_os = "linux")]
    let cpu = if auto_cpu_affinity {
        let cpu = setup.id.get() - 1; // Worker ids start at 1, cpus at 0.
        let cpu_set = cpu_set(cpu);
        match set_affinity(&cpu_set) {
            Ok(()) => {
                debug!("worker thread using CPU '{}'", cpu);
                Some(cpu)
            }
            Err(err) => {
                warn!("error setting CPU affinity: {}", err);
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let cpu = {
        let _ = auto_cpu_affinity; // Silence unused variables warnings.
        None
    };

    let mut runtime = Runtime::new(
        setup.poll,
        setup.waker_id,
        setup.waker_events,
        receiver,
        shared_internals,
        cpu,
    )
    .map_err(|err| rt::Error::worker(Error::Init(err)))?;

    trace::finish(
        &mut trace_log,
        timing,
        "Initialising the worker thread",
        &[],
    );
    runtime.set_trace_log(trace_log);

    // All setup is done, so we're ready to run the event loop.
    runtime.run_event_loop().map_err(rt::Error::worker)
}

/// Create a cpu set that may only run on `cpu`.
#[cfg(target_os = "linux")]
fn cpu_set(cpu: usize) -> libc::cpu_set_t {
    let mut cpu_set = unsafe { mem::zeroed() };
    unsafe { libc::CPU_ZERO(&mut cpu_set) };
    unsafe { libc::CPU_SET(cpu % libc::CPU_SETSIZE as usize, &mut cpu_set) };
    cpu_set
}

/// Set the affinity of this thread to the `cpu_set`.
#[cfg(target_os = "linux")]
fn set_affinity(cpu_set: &libc::cpu_set_t) -> io::Result<()> {
    let thread = unsafe { libc::pthread_self() };
    let res = unsafe { libc::pthread_setaffinity_np(thread, mem::size_of_val(cpu_set), cpu_set) };
    if res == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Error running a [`Worker`].
#[derive(Debug)]
pub(crate) enum Error {
    /// Error in [`RunningRuntime::init`].
    Init(io::Error),
    /// Error polling [`mio::Poll`].
    Polling(io::Error),
    /// Error receiving message on coordinator channel.
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
            Init(err) => write!(f, "error initialising worker: {}", err),
            Polling(err) => write!(f, "error polling for events: {}", err),
            RecvMsg(err) => write!(f, "error receiving message from coordinator: {}", err),
            ProcessInterrupted => write!(
                f,
                "received process signal, but no receivers for it: stopped running"
            ),
            UserFunction(err) => write!(f, "error running user function: {}", err),
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
