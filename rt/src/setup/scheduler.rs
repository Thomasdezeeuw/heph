//! Scheduler implementation.
//!
//! The scheduler is responsible for managing all processes in the runtime.
//! Keeping track of which processes are ready to run and which are waiting for
//! external events (such as incoming connections) to happen before continuing.
//! Furthermore it needs to determine which process to run next, i.e. ordering
//! of the processes.
//!
//! This is implemented using the following traits:
//!  * [`Scheduler`] is the data structure and algorithm that hold all
//!    processes.
//!  * [`Schedule`] can optionally used by a `Scheduler` to determine how to
//!    order processes.
//!  * [`Process`] represents a single process which can be added to the
//!    `Scheduler`.
//!  * [`SchedulerProcess`] represents a process within the context of a
//!    `Scheduler` that can be run.
//!
//! See the documentation on the trait themselves for more information.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, thread};

use heph::{ActorFuture, NewActor, Supervisor};

use crate::panic_message;
use crate::spawn::options::Priority;

/// Scheduler implementation.
///
/// This trait defines the data structure and algorithm uses to manage all
/// thread-local processes for a single worker in the runtime. Generally
/// speaking, the scheduler keep track of two things: 1) all inactive processes
/// (those that are not ready to run) and 2) a ready queue of processes that are
/// ready to run.
///
/// The scheduler can optionally use the [`Schedule`] trait to allow for a user
/// defined scheduling implementation.
pub trait Scheduler: fmt::Debug {
    /// Process that is ready to run.
    type Process: SchedulerProcess + Unpin;

    /// Returns the next process that is ready to run, if any, as well as waker
    /// for it.
    ///
    /// After a process is removed it's run without holding any references to
    /// the scheduler itself, this way it can be used (mutably) to add new
    /// processes to it while running the returned process.
    ///
    /// Once the process is done running either [`Scheduler::add_back_process`]
    /// or [`Scheduler::complete_process`] is called, to ensure that the
    /// scheduler can keep managing the process or clean up any resources
    /// related to it.
    fn next_process(&mut self) -> Option<(Self::Process, task::Waker)>;

    /// Add back a `process` that was run.
    ///
    /// The process was previously removed via [`Scheduler::next_process`] and
    /// now is added back to the scheduler as it's not finished running (i.e. it
    /// returned [`Poll::Pending`]).
    ///
    /// The `stats` about the running of the process can be used for scheduling.
    fn add_back_process(&mut self, process: Self::Process, stats: RunStats);

    /// Mark a `process` as completed.
    ///
    /// The process was previously removed via [`Scheduler::next_process`], was
    /// run to completion. This should clean up any lingering resource related
    /// to the process within the scheduler.
    ///
    /// After any resources within the scheduler are cleaned up, the process
    /// should be dropped. Doing so should be done using [`catch_unwind`] to
    /// catch any panics. The result of dropping the process is expected as
    /// return type.
    ///
    /// **The scheduler should not be affected if dropping the process panics**.
    ///
    /// The default implemementation simply drops the process (catching any
    /// panics).
    fn complete_process(&mut self, process: Self::Process) -> thread::Result<()> {
        // Don't want to panic when dropping the process.
        catch_unwind(AssertUnwindSafe(|| drop(process)))
    }

    // TODO: Currently LocalRuntimeData::add_local_process uses `Pin<Box<dyn
    // Process>>` as process. See if this can be changed (hard to do with dyn
    // traits). Otherwise maybe change it here? For now add_boxed_process is
    // used as a work around to not allocate twice in the scheduler.

    /// Add a new process to the scheduler.
    fn add_process<P>(&mut self, priority: Priority, process: P) -> ProcessId
    where
        P: Process + 'static;

    /// Specialisation for adding boxed processes. Aiming to get rid of the box
    /// and this method.
    #[doc(hidden)]
    fn add_boxed_process(
        &mut self,
        priority: Priority,
        process: Pin<Box<dyn Process>>,
    ) -> ProcessId {
        self.add_process(priority, process)
    }

    /// Mark all processes that are awoken as ready.
    ///
    /// Returns the number of processes that were awoken.
    // TODO: better name, not sure if it's clear enough. Maybe rename/remove the
    // "process" part.
    fn process_wakeups(&mut self) -> usize;

    /// Returns the number of processes that are ready to run.
    fn processes_ready(&self) -> usize;

    /// Returns true if the scheduler has any processes that are ready to run.
    fn has_ready_process(&self) -> bool {
        self.processes_ready() >= 1
    }

    /// Returns the number of processes that are inactive (not ready to run).
    fn processes_inactive(&self) -> usize;

    /// Returns true if the scheduler has any processes (in any state).
    // TODO: how to deal with system processes for user implementations?
    fn has_process(&self) -> bool {
        self.has_ready_process() || self.processes_inactive() >= 1
    }
}

/// Scheduling implementation.
///
/// The [`Scheduler`] trait defines the scheduler itself, i.e. it's data
/// structure and algorithms. This trait can be optionally used in those
/// implementations to determine which process to run next, i.e. the ordering of
/// processes that are ready to run.
///
/// The type itself holds the per process metadata needed for scheduling.
pub trait Schedule {
    /// Create new process metadata.
    fn new(priority: Priority) -> Self;

    /// Update the process data with the latest run information.
    ///
    /// This should called be after the process is added back to the scheduler
    /// in [`Scheduler::add_back_process`] so that the process metadata stays up
    /// to date.
    fn update(&mut self, stats: &RunStats);

    /// Determine if the `lhs` or `rhs` should run first.
    ///
    /// This is essentially the same as [`Ord::cmp`].
    ///
    /// [`Ord::cmp`]: std::cmp::Ord::cmp
    fn order(lhs: &Self, rhs: &Self) -> std::cmp::Ordering;
}

/// Completely Fair Scheduler (CFS) algorithm.
///
/// Implements [`Schedule`] so it can be used in different [`Scheduler`]s. See
/// <https://en.wikipedia.org/wiki/Completely_Fair_Scheduler> for more
/// information about CFS.
#[derive(Debug)]
pub struct Cfs {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
}

impl Schedule for Cfs {
    fn new(priority: Priority) -> Cfs {
        Cfs {
            priority,
            fair_runtime: Duration::ZERO,
        }
    }

    fn update(&mut self, stats: &RunStats) {
        self.fair_runtime += stats.elapsed() * self.priority;
    }

    fn order(lhs: &Self, rhs: &Self) -> std::cmp::Ordering {
        (rhs.fair_runtime)
            .cmp(&(lhs.fair_runtime))
            .then_with(|| lhs.priority.cmp(&rhs.priority))
    }
}

impl Cfs {
    /// Returns the calculated fair runtime.
    #[cfg(any(test, feature = "test"))]
    pub fn fair_runtime(&mut self) -> Duration {
        self.fair_runtime
    }
}

/// Pollable process.
///
/// A process that can be added to a [`Scheduler`], see
/// [`Scheduler::add_process`].
pub trait Process: Future<Output = ()> {
    /// Return the name of this process.
    fn name(&self) -> &'static str;
}

impl<T> Process for Pin<T>
where
    T: std::ops::DerefMut<Target: Process>, // NOTE: DerefMut is required for Future impl.
{
    fn name(&self) -> &'static str {
        (&**self).name()
    }
}

impl<T> Process for Box<T>
where
    T: Process + Unpin + ?Sized, // NOTE: Unpin is required for Future impl.
{
    fn name(&self) -> &'static str {
        (&**self).name()
    }
}

/// Wrapper around a [`Future`] to implement [`Process`].
///
/// NOTE: this type only exists because we can add a default implementation for
/// Fut where Fut: Future, and have a separate one for ActorFuture. Once that
/// kind of specialisation is possible this type can be remove.
pub(crate) struct FutureProcess<Fut>(pub(crate) Fut);

impl<Fut> Process for FutureProcess<Fut>
where
    Fut: Future<Output = ()>,
{
    fn name(&self) -> &'static str {
        // TODO: improve this using `heph::actor::name::<Fut>()`.
        "FutureProcess"
    }
}

impl<Fut> Future for FutureProcess<Fut>
where
    Fut: Future<Output = ()>,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the future.
        unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.0) }.poll(ctx)
    }
}

impl<S, NA> Process for ActorFuture<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: Clone,
{
    fn name(&self) -> &'static str {
        NA::name()
    }
}

/// Process that is part of a [`Scheduler`].
pub trait SchedulerProcess: Process {
    /// Id of the process.
    fn id(&self) -> ProcessId;

    /// Run the process until it can't make any more progress without blocking.
    ///
    /// # Notes
    ///
    /// This calls the [`Future::poll`] function underneath, which means all
    /// limitations that apply to that function also apply here (such as not
    /// calling this after it returns [`Poll::Ready`]).
    fn run(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> RunStats {
        let pid = self.id();
        let name = self.name();
        log::trace!(pid, name; "running process");
        let start = Instant::now();
        let result = match catch_unwind(AssertUnwindSafe(|| self.as_mut().poll(ctx))) {
            Ok(result) => result,
            Err(panic) => {
                let msg = panic_message(&*panic);
                let name = self.name();
                log::error!("process '{name}' panicked at '{msg}'");
                Poll::Ready(())
            }
        };
        let end = Instant::now();
        let elapsed = end - start;
        log::trace!(pid, name, elapsed:?, result:?; "finished running process");
        RunStats {
            start,
            end,
            elapsed,
            result,
        }
    }
}

impl<T> SchedulerProcess for Pin<T>
where
    T: std::ops::DerefMut<Target: SchedulerProcess>, // NOTE: DerefMut is required for Future impl.
{
    fn id(&self) -> ProcessId {
        (&**self).id()
    }
}

impl<T> SchedulerProcess for Box<T>
where
    T: SchedulerProcess + Unpin + ?Sized, // NOTE: Unpin is required for Future impl.
{
    fn id(&self) -> ProcessId {
        (&**self).id()
    }
}

/// Process id, or pid for short, is an identifier for a process in the
/// scheduler.
///
/// This can only be created by one of the schedulers and should be seen as an
/// opaque type otherwise. The id must be unique within the scheduler while the
/// process is alive (after it's dropped the id may be reused).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ProcessId(usize);

impl ProcessId {
    /// Create a new process id.
    pub const fn new(data: usize) -> ProcessId {
        ProcessId(data)
    }

    /// Returns the data passed to [`ProcessId::new`].
    pub const fn data(self) -> usize {
        self.0
    }
}

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[doc(hidden)] // Not part of the stable API.
impl log::kv::ToValue for ProcessId {
    fn to_value(&self) -> log::kv::Value<'_> {
        self.0.to_value()
    }
}

/// Statistics about the run of a process.
#[derive(Copy, Clone, Debug)]
#[must_use = "Must check the process's result"]
pub struct RunStats {
    start: Instant,
    end: Instant,
    elapsed: Duration,
    result: Poll<()>,
}

impl RunStats {
    /// Create new run statictics.
    ///
    /// `end` must before `start` otherwise this will panic.
    #[cfg(any(test, feature = "test"))]
    pub fn new(start: Instant, end: Instant, result: Poll<()>) -> RunStats {
        let elapsed = end.duration_since(start);
        RunStats {
            start,
            end,
            elapsed,
            result,
        }
    }

    /// When the processes started running.
    pub fn start(&self) -> Instant {
        self.start
    }

    /// When the processes finished running.
    pub fn end(&self) -> Instant {
        self.end
    }

    /// How long the process ran for.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub(crate) fn result(&self) -> Poll<()> {
        self.result
    }
}
