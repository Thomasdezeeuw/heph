//! Tracing facilities.

use std::time::Instant;

/// Structure to keep track on what time is spend.
#[derive(Debug)]
pub(crate) struct TimeSpend<T> {
    start: Timestamp,
    events: Vec<Event<T>>,
}

impl<T> TimeSpend<T> {
    /// Create a new `TimeSpend`.
    pub(crate) fn new() -> TimeSpend<T> {
        TimeSpend {
            start: Timestamp::now(),
            events: Vec::new(),
        }
    }

    /// Mark the end of a phase.
    pub(crate) fn mark_end(&mut self, event: T) {
        let event = Event {
            kind: event,
            end: Timestamp::now(),
        };
        self.events.push(event);
    }
}

#[derive(Debug)]
struct Event<T> {
    kind: T,
    end: Timestamp,
}

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
