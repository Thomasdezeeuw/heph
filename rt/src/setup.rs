//! Module with [`Setup`].

use std::cmp::max;
use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::num::NonZeroUsize;
use std::path::{self, Path};
use std::sync::Arc;
use std::{env, fmt, io, thread};

use heph::actor_ref::ActorGroup;
use log::{debug, warn};

use crate::trace;
use crate::wakers::shared::Wakers;
use crate::{coordinator, shared, worker, Error, Runtime};

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
                warn!(
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
        let timing = trace::start(&trace_log);

        let name = name.unwrap_or_else(default_app_name).into_boxed_str();
        debug!(name = name, workers = threads; "building Heph runtime");

        let coordinator_setup = coordinator::setup(name, threads)?;
        let coordinator_sq = coordinator_setup.submission_queue();

        // Setup the worker threads, but don't spawn them yet.
        let mut worker_setups = Vec::with_capacity(threads);
        let mut worker_sqs = Vec::with_capacity(threads);
        for id in 1..=threads {
            // Coordinator has id 0.
            let id = NonZeroUsize::new(id).unwrap();
            let (worker_setup, worker_sq) = worker::setup(id, auto_cpu_affinity, coordinator_sq)
                .map_err(Error::start_worker)?;
            worker_setups.push(worker_setup);
            worker_sqs.push(worker_sq);
        }

        // Create the internal data structure shared by all threads.
        #[allow(clippy::cast_possible_truncation)]
        let entries = max((threads * 64) as u32, 8);
        let setup = shared::RuntimeInternals::setup(coordinator_sq.clone(), entries)
            .map_err(Error::init_coordinator)?;
        let worker_sqs = worker_sqs.into_boxed_slice();
        let shared_trace_log = trace_log.as_ref().map(trace::CoordinatorLog::clone_shared);
        let internals = Arc::new_cyclic(|shared_internals| {
            let wakers = Wakers::new(shared_internals.clone());
            setup.complete(wakers, worker_sqs, shared_trace_log)
        });

        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Setting up the coordinator",
            &[],
        );

        // Spawn the worker threads.
        let timing = trace::start(&trace_log);
        let workers = worker_setups
            .into_iter()
            .map(|worker_setup| {
                #[allow(clippy::cast_possible_truncation)]
                let trace_log = trace_log
                    .as_ref()
                    .map(|trace_log| trace_log.new_stream(worker_setup.id() as u32));
                worker_setup.start(internals.clone(), auto_cpu_affinity, trace_log)
            })
            .collect::<io::Result<Vec<worker::Handle>>>()
            .map_err(Error::start_worker)?;
        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "Spawning worker threads",
            &[("amount", &threads)],
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

// Setup functions used by `coordinator`.

/// Returns (OS name and version, hostname).
///
/// Uses `uname(2)`.
pub(crate) fn host_info() -> io::Result<(Box<str>, Box<str>)> {
    // NOTE: we could also use `std::env::consts::OS`, but this looks better.
    #[cfg(target_os = "linux")]
    const OS: &str = "GNU/Linux";
    #[cfg(target_os = "freebsd")]
    const OS: &str = "FreeBSD";
    #[cfg(target_os = "macos")]
    const OS: &str = "macOS";

    let mut uname_info: MaybeUninit<libc::utsname> = MaybeUninit::uninit();
    if unsafe { libc::uname(uname_info.as_mut_ptr()) } == -1 {
        let os_err = io::Error::last_os_error();
        return Err(io::Error::new(
            os_err.kind(),
            format!("failed to get OS information: {os_err}"),
        ));
    }

    // SAFETY: call to `uname(2)` above ensures `uname_info` is initialised.
    let uname_info = unsafe { uname_info.assume_init() };
    let sysname = unsafe { CStr::from_ptr(uname_info.sysname.as_ptr().cast()).to_string_lossy() };
    let release = unsafe { CStr::from_ptr(uname_info.release.as_ptr().cast()).to_string_lossy() };
    let version = unsafe { CStr::from_ptr(uname_info.version.as_ptr().cast()).to_string_lossy() };
    let nodename = unsafe { CStr::from_ptr(uname_info.nodename.as_ptr().cast()).to_string_lossy() };

    let os = format!("{OS} ({sysname} {release} {version})").into_boxed_str();
    let hostname = nodename.into_owned().into_boxed_str();
    Ok((os, hostname))
}

/// Universally Unique IDentifier (UUID), see [RFC 4122].
///
/// [RFC 4122]: https://datatracker.ietf.org/doc/html/rfc4122
#[derive(Copy, Clone)]
#[allow(clippy::doc_markdown)]
pub(crate) struct Uuid(u128);

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

/// Get the host id by reading `/etc/machine-id` on Linux or `/etc/hostid` on
/// FreeBSD.
#[cfg(any(target_os = "freebsd", target_os = "linux"))]
pub(crate) fn host_id() -> io::Result<Uuid> {
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
        io::Error::new(
            err.kind(),
            format!("failed to get host id: can't open '{PATH}': {err}"),
        )
    })?;

    if n == EXPECTED_SIZE {
        #[cfg(target_os = "linux")]
        let res = from_hex(&buf[..EXPECTED_SIZE]);
        #[cfg(target_os = "freebsd")]
        let res = from_hex_hyphenated(&buf[..EXPECTED_SIZE]);

        res.map_err(|()| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to get host id: invalid '{PATH}' format: input is not hex"),
            )
        })
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to get host id: can't read '{PATH}', invalid format: only read {n} bytes (expected {EXPECTED_SIZE})"),
        ))
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
pub(crate) fn host_id() -> io::Result<Uuid> {
    let mut bytes = [0; 16];
    let timeout = libc::timespec {
        tv_sec: 1, // This shouldn't block, but just in case. SQLite does this also.
        tv_nsec: 0,
    };
    if unsafe { libc::gethostuuid(bytes.as_mut_ptr(), &timeout) } == -1 {
        let os_err = io::Error::last_os_error();
        Err(io::Error::new(
            os_err.kind(),
            format!("failed to get host id: {os_err}"),
        ))
    } else {
        Ok(Uuid(u128::from_be_bytes(bytes)))
    }
}

// Setup functions used by `worker`.

/// Set thread's CPU affinity.
pub(crate) fn set_cpu_affinity(worker_id: NonZeroUsize) -> Option<usize> {
    #[cfg(not(target_os = "linux"))]
    {
        _ = worker_id; // Silence unused variables warnings.
        None
    }

    #[cfg(target_os = "linux")]
    {
        let cpu = worker_id.get() - 1; // Worker ids start at 1, cpus at 0.
        let cpu_set = cpu_set(cpu);
        match set_affinity(&cpu_set) {
            Ok(()) => {
                debug!(worker_id = worker_id; "worker thread CPU affinity set to {cpu}");
                Some(cpu)
            }
            Err(err) => {
                warn!(worker_id = worker_id; "failed to set CPU affinity on thread: {err}");
                None
            }
        }
    }
}

/// Create a cpu set that may only run on `cpu`.
#[cfg(target_os = "linux")]
fn cpu_set(cpu: usize) -> libc::cpu_set_t {
    let mut cpu_set = unsafe { std::mem::zeroed() };
    unsafe { libc::CPU_ZERO(&mut cpu_set) };
    unsafe { libc::CPU_SET(cpu % libc::CPU_SETSIZE as usize, &mut cpu_set) };
    cpu_set
}

/// Set the affinity of this thread to the `cpu_set`.
#[cfg(target_os = "linux")]
fn set_affinity(cpu_set: &libc::cpu_set_t) -> io::Result<()> {
    let thread = unsafe { libc::pthread_self() };
    let res = unsafe { libc::pthread_setaffinity_np(thread, size_of_val(cpu_set), cpu_set) };
    if res == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
