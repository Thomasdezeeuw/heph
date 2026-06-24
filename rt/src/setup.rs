//! Module with [`Setup`].

use std::num::NonZeroUsize;
use std::path::{self, Path};
use std::time::Duration;
use std::{env, thread};

use heph::actor_ref::ActorGroup;

use crate::rt::Timers;
use crate::timing_wheel::TimingWheel;
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
pub struct Setup<FT = DefaultTimers> {
    /// Name of the application.
    name: Option<String>,
    /// Number of worker threads to create.
    threads: usize,
    /// Function to create a [`Timers`] implementation per worker thread.
    create_timers: FT,
    /// Whether or not to automatically set CPU affinity.
    auto_cpu_affinity: bool,
    /// Number of processes to run in between calls to poll.
    run_poll_ratio: usize,
    /// Target time for the duration of a single iteration of the event loop.
    ///
    /// If the event loop iteration elapses this timeout no more processes are
    /// run, regardless of how many have run so far.
    max_run_time: Duration,
    /// Optional trace log.
    trace_log: Option<trace::CoordinatorLog>,
}

impl Setup {
    /// See [`Runtime::setup`].
    pub(crate) const fn new() -> Setup {
        Setup {
            name: None,
            threads: 1,
            create_timers: TimingWheel::new,
            auto_cpu_affinity: false,
            run_poll_ratio: 32,
            max_run_time: Duration::from_millis(5),
            trace_log: None,
        }
    }
}

impl<FT, T> Setup<FT>
where
    FT: FnOnce() -> T + Clone + Send + 'static,
    T: Timers + 'static,
{
    /// Set the name of the application.
    ///
    /// If the name is not set when the runtime is build the name of the binary
    /// called will be used.
    pub fn with_name(mut self, name: String) -> Self {
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

    /// Change the thread-local [`Timers`] implementation.
    ///
    /// The function `create_timers` will be created on each worker to create a
    /// new [`Timers`] implementation for each thread.
    ///
    /// Defaults to [`TimingWheel`].
    ///
    /// # Notes
    ///
    /// This doesn't change the implementation for thread-safe timers.
    pub fn with_timers<FT2, T2>(self, create_timers: FT2) -> Setup<FT2>
    where
        FT2: FnOnce() -> T2 + Clone + Send + 'static,
        T2: Timers + 'static,
    {
        #[rustfmt::skip]
        let Setup { name, threads, create_timers: _, auto_cpu_affinity, run_poll_ratio, max_run_time, trace_log } = self;
        Setup {
            name,
            threads,
            create_timers,
            auto_cpu_affinity,
            run_poll_ratio,
            max_run_time,
            trace_log,
        }
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

    /// Maximum number of processes to run in between polling for more events
    /// (such as I/O events).
    ///
    /// Defaults to 32.
    ///
    /// # Notes
    ///
    /// Also see [`Setup::with_max_run_time`] to change the maximum duration
    /// between polls.
    pub const fn with_run_poll_ratio(mut self, ratio: usize) -> Self {
        self.run_poll_ratio = ratio;
        self
    }

    /// Maximum time to run processes, before polling for more events.
    ///
    /// If the event loop iteration elapses this timeout no more processes are
    /// run, regardless of how many have run so far and instead we poll for more
    /// events.
    ///
    /// Defaults to 5 milliseconds.
    ///
    /// # Notes
    ///
    /// Also see [`Setup::with_run_poll_ratio`] to change the maximum number of
    /// processes run between polls.
    pub const fn with_max_run_time(mut self, timeout: Duration) -> Self {
        self.max_run_time = timeout;
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
        let Setup { name, threads, create_timers, auto_cpu_affinity, run_poll_ratio, max_run_time, mut trace_log } = self;
        let timing = trace::start(&trace_log);

        let name = name.unwrap_or_else(default_app_name).into_boxed_str();
        log::trace!(name, workers = threads; "building Heph runtime");

        // Setup the coordinator, but we can't fully construct Coordinator until
        // we spawned all synchronous actors (which is done using the returned
        // Runtime).
        let coordinator_setup = coordinator::setup().map_err(Error::init_coordinator)?;
        let coordinator_sq = coordinator_setup.sq();

        // Create the internal data structure shared by all threads.
        let shared_log = trace_log.as_ref().map(trace::CoordinatorLog::clone_shared);
        let internals = shared::RuntimeInternals::new(name, coordinator_sq, shared_log)
            .map_err(Error::init_coordinator)?;

        // Spawn the worker threads.
        let mut spawned_workers = Vec::with_capacity(threads);
        for id in 1..=threads {
            let conf = worker::Conf {
                // Coordinator has id 0.
                id: NonZeroUsize::new(id).unwrap(),
                shared_internals: internals.clone(),
                create_timers: create_timers.clone(),
                auto_cpu_affinity,
                run_poll_ratio,
                max_run_time,
            };
            let spawned_worker = worker::spawn_thread(conf).map_err(Error::start_worker)?;
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

/// Uses [`TimingWheel`] as default timers implementation.
type DefaultTimers = fn() -> TimingWheel;
