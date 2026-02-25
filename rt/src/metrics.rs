//! Runtime metrics.
//!
//! See [`SharedMetrics`] and [`RuntimeRef::shared_metrics`]. And
//! [`LocalMetrics`] and [`RuntimeRef::local_metrics`].
//!
//! [`RuntimeRef::shared_metrics`]: crate::RuntimeRef::shared_metrics
//! [`RuntimeRef::local_metrics`]: crate::RuntimeRef::local_metrics

use std::time::{Duration, Instant};

use crate::cpu_usage;

/// Shared metrics.
///
/// This includes metrics about thread-safe actors and anything that is shared
/// between worker threads. For metrics for thread-local actors and anything
/// worker thread specific see [`LocalMetrics`].
#[derive(Debug)]
pub struct SharedMetrics {
    /// Time when the runtime started.
    pub(crate) start: Instant,
    pub(crate) scheduler_ready: usize,
    pub(crate) scheduler_inactive: usize,
    pub(crate) timers: usize,
    pub(crate) timers_next: Option<Duration>,
    pub(crate) trace_counter: usize,
}

impl SharedMetrics {
    /// Duration since the starting of the runtime.
    pub fn uptime(&self) -> Duration {
        self.start.elapsed()
    }

    /// CPU time consumed by the process.
    pub fn total_cpu_time(&self) -> Duration {
        cpu_usage(libc::CLOCK_PROCESS_CPUTIME_ID)
    }

    /// Number of shared processes that are ready to run.
    pub fn scheduler_ready(&self) -> usize {
        self.scheduler_ready
    }

    /// Number of shared processes that are inactive.
    pub fn scheduler_inactive(&self) -> usize {
        self.scheduler_inactive
    }

    /// Number of shared timers.
    pub fn timers(&self) -> usize {
        self.timers
    }

    /// Next shared timer to expire, if any.
    pub fn timers_next(&self) -> Option<Duration> {
        self.timers_next
    }

    /// Number of events genered by running thread-safe actors.
    pub fn trace_counter(&self) -> usize {
        self.trace_counter
    }
}

/// Local metrics for a worker thread.
///
/// This include information about thread-local actors.
#[derive(Debug)]
pub struct LocalMetrics {
    pub(crate) scheduler_ready: usize,
    pub(crate) scheduler_inactive: usize,
    pub(crate) timers: usize,
    pub(crate) timers_next: Option<Duration>,
    pub(crate) trace_counter: usize,
}

impl LocalMetrics {
    /// CPU time consumed by the thread.
    pub fn cpu_time(&self) -> Duration {
        cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID)
    }

    /// Number of shared processes that are ready to run.
    pub fn scheduler_ready(&self) -> usize {
        self.scheduler_ready
    }

    /// Number of shared processes that are inactive.
    pub fn scheduler_inactive(&self) -> usize {
        self.scheduler_inactive
    }

    /// Number of shared timers.
    pub fn timers(&self) -> usize {
        self.timers
    }

    /// Next shared timer to expire, if any.
    pub fn timers_next(&self) -> Option<Duration> {
        self.timers_next
    }

    /// Number of events genered by running thread-safe actors.
    pub fn trace_counter(&self) -> usize {
        self.trace_counter
    }
}
