//! Module with [`rt::Setup`].
//!
//! [`rt::Setup`]: Setup

use std::num::NonZeroUsize;
use std::path::Path;
use std::{io, thread};

use log::{debug, warn};

use crate::actor_ref::ActorGroup;
use crate::rt::trace;
use crate::rt::{coordinator, Coordinator};
use crate::rt::{Error, Runtime, Worker, MAX_THREADS};

/// Setup a [`Runtime`].
///
/// This type implements a builder pattern to build a `Runtime`. It is created
/// via [`Runtime::setup`], for examples and usage see [`Runtime`].
#[derive(Debug)]
pub struct Setup {
    /// Number of worker threads to create.
    threads: usize,
    /// Whether or not to automatically set CPU affinity.
    auto_cpu_affinity: bool,
    /// Optional trace log.
    trace_log: Option<trace::Log>,
}

impl Setup {
    /// See [`Runtime::setup`].
    pub(super) const fn new() -> Setup {
        Setup {
            threads: 1,
            auto_cpu_affinity: false,
            trace_log: None,
        }
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
    pub fn get_threads(&self) -> usize {
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
    pub fn auto_cpu_affinity(mut self) -> Self {
        self.auto_cpu_affinity = true;
        self
    }

    /// Generate a trace of the runtime, writing it to the file specified by
    /// `path`.
    ///
    /// See the [`mod@trace`] module for more information.
    pub fn enable_tracing<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        match trace::Log::open(path.as_ref(), coordinator::TRACE_ID) {
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
    pub fn build(mut self) -> Result<Runtime, Error> {
        debug!("building Heph runtime: worker_threads={}", self.threads);

        let coordinator = Coordinator::init().map_err(Error::init_coordinator)?;

        // Start our worker threads.
        let timing = trace::start(&self.trace_log);
        let workers = (1..=self.threads)
            .map(|id| {
                let id = NonZeroUsize::new(id).unwrap();
                let trace_log = if let Some(trace_log) = &self.trace_log {
                    Some(trace_log.new_stream(id.get() as u32)?)
                } else {
                    None
                };
                Worker::start(
                    id,
                    coordinator.shared_internals().clone(),
                    self.auto_cpu_affinity,
                    trace_log,
                )
            })
            .collect::<io::Result<Vec<Worker>>>()
            .map_err(Error::start_worker)?;
        trace::finish(
            &mut self.trace_log,
            timing,
            "Spawning worker threads",
            &[("amount", &self.threads)],
        );

        Ok(Runtime {
            coordinator,
            workers,
            sync_actors: Vec::new(),
            signals: ActorGroup::empty(),
            trace_log: self.trace_log,
        })
    }
}
