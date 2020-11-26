//! Tracing facilities.

use std::time::Instant;

/// A collection of [`TimeSpan`]s.
#[derive(Debug)]
pub(crate) struct TimeSpans {
    // TODO: limit the amount of timespans kept in memory.
    spans: Vec<TimeSpan>,
}

impl TimeSpans {
    /// Creates an empty collection.
    pub(crate) const fn new() -> TimeSpans {
        TimeSpans { spans: Vec::new() }
    }

    /// Add a new [`TimeSpan`], with a `start`ing and `end`ing point.
    pub(crate) fn add(&mut self, start: Timestamp, end: Timestamp) {
        let span = TimeSpan::new(start, end);
        self.add_span(span);
    }

    /// Add a new [`TimeSpan`].
    pub(crate) fn add_span(&mut self, span: TimeSpan) {
        self.spans.push(span)
    }
}

/// A span of time, in other words two [`Timestamp`]s.
#[derive(Debug)]
pub(crate) struct TimeSpan {
    start: Timestamp,
    end: Timestamp,
}

impl TimeSpan {
    /// Create a new [`TimeSpan`].
    pub(crate) const fn new(start: Timestamp, end: Timestamp) -> TimeSpan {
        TimeSpan { start, end }
    }
}

/// A point in time.
pub(crate) type Timestamp = Instant;
