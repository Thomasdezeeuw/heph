//! Worker thread code.
//!
//! A worker thread manages part of the [`Runtime`]. It manages two parts; the
//! local and shared (between workers) parts of the runtime. The local part
//! include thread-local actors and futures, timers for those local actors, I/O
//! state, etc. The can be found in [`local::Runtime`]. The shared part is
//! similar, but not the sole responsibility of a single worker, all workers
//! collectively are responsible for it. This shared part can be fore in
//! [`shared::RuntimeInternals`].
//!
//! Creating a new worker starts with calling [`setup`] to prepare various
//! things that need to happen on the main/coordinator thread. After that worker
//! thread can be [started], which runs [`main`] in a new thread.
//!
//! [`Runtime`]: crate::rt::Runtime
//! [`local::Runtime`]: crate::rt::local::Runtime
//! [started]: WorkerSetup::start

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{io, thread};

use crossbeam_channel::{self, Receiver};
use mio::{Poll, Registry, Token};

use crate::rt::local::waker::{self, WakerId};
use crate::rt::local::{Control, Runtime, WAKER};
use crate::rt::setup::set_cpu_affinity;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{self, shared, ProcessId, RuntimeRef, Signal};
use crate::trace;

pub(super) use crate::rt::local::Error;

/// Setup a new worker thread.
///
/// Use [`WorkerSetup::start`] to spawn the worker thread.
pub(super) fn setup(id: NonZeroUsize) -> io::Result<(WorkerSetup, &'static ThreadWaker)> {
    let poll = Poll::new()?;

    // Setup the waking mechanism.
    let (waker_sender, waker_events) = crossbeam_channel::unbounded();
    let waker = mio::Waker::new(poll.registry(), WAKER)?;
    let waker_id = waker::init(waker, waker_sender);
    let thread_waker = waker::get_thread_waker(waker_id);

    let setup = WorkerSetup {
        id,
        poll,
        waker_id,
        waker_events,
    };
    Ok((setup, thread_waker))
}

/// Setup work required before starting a worker thread, see [`setup`].
pub(super) struct WorkerSetup {
    /// See [`Worker::id`].
    id: NonZeroUsize,
    /// Poll instance for the worker thread. This is needed before starting the
    /// thread to initialise the [`rt::local::waker`].
    poll: Poll,
    /// Waker id used to create a `Waker` for thread-local actors.
    waker_id: WakerId,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
}

impl WorkerSetup {
    /// Start a new worker thread.
    pub(super) fn start(
        self,
        shared_internals: Arc<shared::RuntimeInternals>,
        auto_cpu_affinity: bool,
        trace_log: Option<trace::Log>,
    ) -> io::Result<Worker> {
        rt::channel::new().and_then(|(channel, receiver)| {
            // Copy id to move into `Worker`, `self` moves into the spawned
            // thread.
            let id = self.id;
            thread::Builder::new()
                .name(format!("Worker {}", id))
                .spawn(move || {
                    main(
                        self,
                        receiver,
                        shared_internals,
                        auto_cpu_affinity,
                        trace_log,
                    )
                })
                .map(|handle| Worker {
                    id,
                    channel,
                    handle,
                })
        })
    }

    /// Return the worker's id.
    pub(super) const fn id(&self) -> usize {
        self.id.get()
    }
}

/// Handle to a worker thread.
#[derive(Debug)]
pub(super) struct Worker {
    /// Unique id (among all threads in the [`rt::Runtime`]).
    id: NonZeroUsize,
    /// Two-way communication channel to share messages with the worker thread.
    channel: rt::channel::Sender<Control>,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), rt::Error>>,
}

impl Worker {
    /// Return the worker's id.
    pub(super) const fn id(&self) -> usize {
        self.id.get()
    }

    /// Registers the channel used to communicate with the thread. Uses the
    /// [`Worker::id`] as [`Token`].
    pub(super) fn register(&mut self, registry: &Registry) -> io::Result<()> {
        self.channel.register(registry, Token(self.id()))
    }

    /// Send the worker thread a signal that the runtime has started.
    pub(super) fn send_runtime_started(&mut self) -> io::Result<()> {
        self.channel.try_send(Control::Started)
    }

    /// Send the worker thread a `signal`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        self.channel.try_send(Control::Signal(signal))
    }

    /// Send the worker thread the function `f` to run.
    pub(super) fn send_function(
        &mut self,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static>,
    ) -> io::Result<()> {
        self.channel.try_send(Control::Run(f))
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<Result<(), rt::Error>> {
        self.handle.join()
    }
}

/// The main function of a worker thread.
fn main(
    setup: WorkerSetup,
    receiver: rt::channel::Receiver<Control>,
    shared_internals: Arc<shared::RuntimeInternals>,
    auto_cpu_affinity: bool,
    trace_log: Option<trace::Log>,
) -> Result<(), rt::Error> {
    let timing = trace::start(&trace_log);

    let cpu = if auto_cpu_affinity {
        set_cpu_affinity(setup.id)
    } else {
        None
    };

    let mut runtime = Runtime::new(
        setup.id,
        setup.poll,
        setup.waker_id,
        setup.waker_events,
        receiver,
        shared_internals,
        trace_log,
        cpu,
    )
    .map_err(|err| rt::Error::worker(Error::Init(err)))?;

    trace::finish_rt(
        (&mut *runtime.trace_log()).as_mut(),
        timing,
        "Initialising the worker thread",
        &[],
    );

    // All setup is done, so we're ready to run the event loop.
    runtime.run_event_loop().map_err(rt::Error::worker)
}
