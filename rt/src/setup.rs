//! Module with [`Setup`].

use std::num::NonZeroUsize;
use std::path::{self, Path};
use std::{env, thread};

use heph::actor_ref::ActorGroup;

use crate::trace;
use crate::{Error, Runtime, coordinator, shared, worker};

/// Setup a [`Runtime`].
///
/// This type implements a builder pattern to build a `Runtime`. It is created
/// via [`Runtime::setup`], for examples and usage see [crate documentation].
///
/// [crate documentation]: crate#running-hephs-runtime
#[derive(Debug)]
#[must_use = "`heph_rt::Setup` doesn't do anything until its `build`"]
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
    pub(crate) const fn new() -> Setup {
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
        assert!(!name.is_empty(), "Can't use an empty application name");
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
        assert!(n != 0, "Can't create zero worker threads, 1 is the minimum");
        self.threads = n;
        self
    }

    /// Set the number of worker threads equal to the number of CPU cores.
    ///
    /// This uses [`thread::available_parallelism`], please read its
    /// documentation for a number of caveats and platform-specific behaviour.
    pub fn use_all_cores(self) -> Self {
        let n = match thread::available_parallelism() {
            Ok(n) => n.get(),
            Err(err) => {
                log::warn!(
                    "failed to get the available concurrency: {err}, using a single worker thread",
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
    /// Thread-local workers creating sockets will use [`SO_INCOMING_CPU`] to
    /// set the CPU affinity to the same value as the worker's affinity.
    ///
    /// [`pthread_setaffinity_np(3)`]: https://man7.org/linux/man-pages/man3/pthread_setaffinity_np.3.html
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
        let timing = trace::start(&trace_log);

        let name = name.unwrap_or_else(default_app_name).into_boxed_str();
        log::trace!(name, workers = threads; "building Heph runtime");

        // Setup the coordinator, but we can't fully construct Coordinator until
        // we spawned all synchronous actors (which is done using the returned
        // Runtime).
        let coordinator_setup = coordinator::setup(name).map_err(Error::init_coordinator)?;
        let coordinator_sq = coordinator_setup.sq();

        // Create the internal data structure shared by all threads.
        let shared_log = trace_log.as_ref().map(trace::CoordinatorLog::clone_shared);
        let internals = shared::RuntimeInternals::new(coordinator_sq, shared_log)
            .map_err(Error::init_coordinator)?;

        // Spawn the worker threads.
        let mut spawned_workers = Vec::with_capacity(threads);
        for id in 1..=threads {
            // Coordinator has id 0.
            let id = NonZeroUsize::new(id).unwrap();
            let internals = internals.clone();
            let spawned_worker = worker::spawn_thread(id, internals, auto_cpu_affinity)
                .map_err(Error::start_worker)?;
            spawned_workers.push(spawned_worker);
        }

        // Start the spawned worker threads.
        let workers = spawned_workers
            .into_iter()
            .map(|spawned_worker| spawned_worker.wait_running())
            .collect();

        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Building runtime",
            &[("worker_threads", &threads)],
        );

        Ok(Runtime {
            coordinator_setup,
            internals,
            workers,
            sync_actors: Vec::new(),
            signals: ActorGroup::empty(),
            trace_log,
        })
    }
}

/// Returns the name of the binary called (i.e. `arg[0]`) as name.
fn default_app_name() -> String {
    match env::args().next() {
        Some(mut bin_path) => {
            if let Some(idx) = bin_path.rfind(path::MAIN_SEPARATOR) {
                drop(bin_path.drain(..=idx));
            }
            bin_path
        }
        None => "<unknown>".to_owned(),
    }
}
