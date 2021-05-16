//! Module with [`rt::Setup`].
//!
//! [`rt::Setup`]: Setup

use std::num::NonZeroUsize;
use std::path::Path;
use std::{env, io, thread};

use log::{debug, warn};

use crate::actor_ref::ActorGroup;
use crate::rt::coordinator::Coordinator;
use crate::rt::{worker, Error, Runtime, Worker, MAX_THREADS};
use crate::trace;

/// Setup a [`Runtime`].
///
/// This type implements a builder pattern to build a `Runtime`. It is created
/// via [`Runtime::setup`], for examples and usage see [`rt`] module.
///
/// [`rt`]: crate::rt
#[derive(Debug)]
pub struct Setup {
    /// Name of the application.
    name: Option<String>,
    /// Number of worker threads to create.
    threads: usize,
    /// Whether or not to automatically set CPU affinity.
    auto_cpu_affinity: bool,
    /// Optional trace log.
    trace_log: Option<trace::CoordinatorLog>,
}

impl Setup {
    /// See [`Runtime::setup`].
    pub(super) const fn new() -> Setup {
        Setup {
            name: None,
            threads: 1,
            auto_cpu_affinity: false,
            trace_log: None,
        }
    }

    /// Set the name of the application.
    ///
    /// If the name is not set when the runtime is build the name of the binary
    /// called will be used.
    pub fn with_name(mut self, name: String) -> Setup {
        if name.is_empty() {
            panic!("Can't use an empty application name");
        }
        self.name = Some(name);
        self
    }

    /// Returns the application name, if set using [`Setup::with_name`].
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set the number of worker threads to use, defaults to one.
    ///
    /// Most applications would want to use [`Setup::use_all_cores`] which sets
    /// the number of threads equal to the number of CPU cores.
    pub fn num_threads(mut self, n: usize) -> Self {
        if n > MAX_THREADS {
            panic!(
                "Can't create {} worker threads, {} is the maximum",
                n, MAX_THREADS
            );
        } else if n == 0 {
            panic!("Can't create zero worker threads, one is the minimum");
        }
        self.threads = n;
        self
    }

    /// Set the number of worker threads equal to the number of CPU cores.
    ///
    /// This uses [`thread::available_concurrency`], please read its
    /// documentation for a number of caveats and platform-specific behaviour.
    pub fn use_all_cores(self) -> Self {
        let n = match thread::available_concurrency() {
            Ok(n) => n.get(),
            Err(err) => {
                warn!(
                    "failed to get the available concurrency: {}, using a single worker thread",
                    err
                );
                1
            }
        };
        self.num_threads(n)
    }

    /// Returns the number of worker threads to use.
    ///
    /// See [`Setup::num_threads`].
    pub const fn get_threads(&self) -> usize {
        self.threads
    }

    /// Automatically set CPU affinity.
    ///
    /// This uses [`pthread_setaffinity_np(3)`] to set the CPU affinity for each
    /// worker thread to there own CPU core.
    ///
    /// Thread-local workers creating sockets, such as [`UdpSocket`] or
    /// [`TcpStream`], will use [`SO_INCOMING_CPU`] to set the CPU affinity to
    /// the same value as the worker's affinity.
    ///
    /// [`pthread_setaffinity_np(3)`]: https://man7.org/linux/man-pages/man3/pthread_setaffinity_np.3.html
    /// [`UdpSocket`]: crate::net::UdpSocket
    /// [`TcpStream`]: crate::net::TcpStream
    /// [`SO_INCOMING_CPU`]: https://man7.org/linux/man-pages/man7/socket.7.html
    ///
    /// # Notes
    ///
    /// The is mostly useful when using [`Setup::use_all_cores`] to create a
    /// single worker thread per CPU core.
    ///
    /// This is currently only implementated on Linux.
    pub const fn auto_cpu_affinity(mut self) -> Self {
        self.auto_cpu_affinity = true;
        self
    }

    /// Generate a trace of the runtime, writing it to the file specified by
    /// `path`.
    ///
    /// See the [`mod@trace`] module for more information.
    ///
    /// Returns an error if a file at `path` already exists or can't create the
    /// file.
    pub fn enable_tracing<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        match trace::CoordinatorLog::open(path.as_ref()) {
            Ok(trace_log) => {
                self.trace_log = Some(trace_log);
                Ok(())
            }
            Err(err) => Err(Error::setup_trace(err)),
        }
    }

    /// Build the runtime.
    ///
    /// This will spawn a number of worker threads (see [`Setup::num_threads`])
    /// to run all the actors.
    pub fn build(self) -> Result<Runtime, Error> {
        #[rustfmt::skip]
        let Setup { name, threads, auto_cpu_affinity, mut trace_log } = self;
        let name = name.unwrap_or_else(default_app_name).into_boxed_str();
        debug!(
            "building Heph runtime: name={}, worker_threads={}",
            name, threads
        );

        // Setup the worker threads.
        let timing = trace::start(&trace_log);
        let mut worker_setups = Vec::with_capacity(threads);
        let mut thread_wakers = Vec::with_capacity(threads);
        for id in 1..=threads {
            // Coordinator has id 0.
            let id = NonZeroUsize::new(id).unwrap();
            let (worker_setup, thread_waker) = worker::setup(id).map_err(Error::start_worker)?;
            worker_setups.push(worker_setup);
            thread_wakers.push(thread_waker);
        }

        // Create the coordinator to oversee all workers.
        let thread_wakers = thread_wakers.into_boxed_slice();
        let shared_trace_log = trace_log.as_ref().map(trace::CoordinatorLog::clone_shared);
        let coordinator = Coordinator::init(name, thread_wakers, shared_trace_log)
            .map_err(Error::init_coordinator)?;

        // Spawn the worker threads.
        let workers = worker_setups
            .into_iter()
            .map(|worker_setup| {
                // See <https://github.com/rust-lang/rust-clippy/issues/6795>.
                #[allow(clippy::manual_map)]
                let trace_log = if let Some(trace_log) = &trace_log {
                    #[allow(clippy::cast_possible_truncation)]
                    Some(trace_log.new_stream(worker_setup.id() as u32))
                } else {
                    None
                };
                worker_setup.start(
                    coordinator.shared_internals().clone(),
                    auto_cpu_affinity,
                    trace_log,
                )
            })
            .collect::<io::Result<Vec<Worker>>>()
            .map_err(Error::start_worker)?;

        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Spawning worker threads",
            &[("amount", &threads)],
        );

        Ok(Runtime {
            coordinator,
            workers,
            sync_actors: Vec::new(),
            signals: ActorGroup::empty(),
            trace_log,
        })
    }
}

/// Returns the name of the binary called (i.e. arg[0]) as name.
fn default_app_name() -> String {
    match env::args().next() {
        Some(mut bin_path) => {
            if let Some(idx) = bin_path.rfind('/') {
                drop(bin_path.drain(..=idx));
            }
            bin_path
        }
        None => "<unknown>".to_string(),
    }
}
