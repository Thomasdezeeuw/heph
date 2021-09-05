//! Facilities for metric collection.
//!
//! Various types in this crate support metric collection. What metrics are
//! collected and how they look are different per type. To see what types
//! support metric collection see the [implementors of Collect].
//!
//! [implementors of Collect]: ./trait.Collect.html#implementors
//!
//! # Two traits and a type (walk into a bar...)
//!
//! The [`Collect`] trait defines how metrics can be collected and from which
//! types metrics can be collected.
//!
//! The [`Metrics`] trait defines how to access the metrics. It returns an
//! iterator that iterates over the metrics in (metric name, metric value)
//! pairs.
//!
//! Finally there is [`Metric`] which is the container type for all metrics.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Collect metrics.
pub trait Collect {
    /// Metrics specific to the type.
    type Metrics: Metrics;

    /// Get the current metrics from this type.
    fn metrics(&self) -> &Self::Metrics;

    /// Get the current metrics wrapped in [`MetricsSource`] allowing them to be
    /// used as [`log::kv::Source`].
    fn metrics_kv(&self) -> MetricsSource<&Self::Metrics> {
        MetricsSource(self.metrics())
    }
}

/// [Collected] metrics.
///
/// [Collected]: Collect
pub trait Metrics: Clone {
    /// Returns a (metric name, metric value) pair.
    type Iter: Iterator<Item = (&'static str, Metric)>;

    /// Returns an iterator that loops over all metrics.
    fn iter(&self) -> Self::Iter;
}

/// Metric container.
///
/// Type that can hold different types of metrics.
#[derive(Debug, Clone)]
pub enum Metric {
    /// Simple counter.
    ///
    /// A counter represent a single monotonic value, which means the value can
    /// only be incremented, not decremented. Only after a restart may it be
    /// reset to zero. Examples of counters are the amount of bytes send or
    /// received on a connection.
    Counter(usize),

    /// Gauge is a Metric that represents a single numerical value that can
    /// arbitrarily go up and down.
    ///
    /// Gauges are typically used for measured values like the number of actors
    /// currently running or the number of concurrent requests.
    Gauge(usize),
}

impl Metric {
    /// Returns `self` as `log::kv::Value`.
    fn to_kv_value(self) -> log::kv::Value<'static> {
        match self {
            Metric::Counter(count) => log::kv::Value::from(count),
            Metric::Gauge(count) => log::kv::Value::from(count),
        }
    }
}

/// Returns [`Metric::Counter`].
impl From<&Counter> for Metric {
    fn from(counter: &Counter) -> Metric {
        Metric::Counter(counter.0)
    }
}

/// Returns [`Metric::Counter`].
impl From<&AtomicCounter> for Metric {
    fn from(counter: &AtomicCounter) -> Metric {
        Metric::Counter(counter.0.load(Ordering::Relaxed))
    }
}

/// Returns [`Metric::Gauge`].
impl From<&AtomicGauge> for Metric {
    fn from(guage: &AtomicGauge) -> Metric {
        Metric::Gauge(guage.0.load(Ordering::Relaxed))
    }
}

/// Simple counter, see [`Metric::Counter`].
#[derive(Debug, Copy, Clone)]
pub(crate) struct Counter(usize);

impl Counter {
    /// Create a new counter starting at zero.
    pub(crate) const fn new() -> Counter {
        Counter(0)
    }

    /// Add `n` to the counter.
    pub(crate) const fn add(&mut self, n: usize) {
        self.0 += n;
    }
}

/// Atomic counter, see [`Metric::Counter`].
#[derive(Debug)]
pub(crate) struct AtomicCounter(AtomicUsize);

impl AtomicCounter {
    /// Create a new counter starting at zero.
    pub(crate) const fn new() -> AtomicCounter {
        AtomicCounter(AtomicUsize::new(0))
    }

    /// Add `n` to the counter.
    pub(crate) fn add(&self, n: usize) {
        let _ = self.0.fetch_add(n, Ordering::Relaxed);
    }
}

impl Clone for AtomicCounter {
    fn clone(&self) -> Self {
        AtomicCounter(AtomicUsize::new(self.0.load(Ordering::Relaxed)))
    }

    fn clone_from(&mut self, source: &Self) {
        *self.0.get_mut() = source.0.load(Ordering::Relaxed);
    }
}

/// Atomic gauge, see [`Metric::Gauge`].
#[derive(Debug)]
pub(crate) struct AtomicGauge(AtomicUsize);

impl AtomicGauge {
    /// Create a new gauge starting at zero.
    pub(crate) const fn new() -> AtomicGauge {
        AtomicGauge(AtomicUsize::new(0))
    }

    /// Add `n` to the counter.
    pub(crate) fn add(&self, n: usize) {
        let _ = self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// Subtract `n` from the counter.
    pub(crate) fn sub(&self, n: usize) {
        let _ = self.0.fetch_sub(n, Ordering::Relaxed);
    }
}

impl Clone for AtomicGauge {
    fn clone(&self) -> Self {
        AtomicGauge(AtomicUsize::new(self.0.load(Ordering::Relaxed)))
    }

    fn clone_from(&mut self, source: &Self) {
        *self.0.get_mut() = source.0.load(Ordering::Relaxed);
    }
}

/// Wrapper around metrics `M` to implement [`log::kv::Source`].
#[derive(Debug, Clone)]
pub struct MetricsSource<M>(pub M);

impl<M> log::kv::Source for MetricsSource<M>
where
    M: Metrics + ?Sized,
{
    fn visit<'kvs>(
        &'kvs self,
        visitor: &mut dyn log::kv::Visitor<'kvs>,
    ) -> Result<(), log::kv::Error> {
        for (name, metric) in self.0.iter() {
            let key = log::kv::Key::from_str(name);
            let value = metric.to_kv_value();
            visitor.visit_pair(key, value)?;
        }
        Ok(())
    }

    fn get<'v>(&'v self, key: log::kv::Key<'_>) -> Option<log::kv::Value<'v>> {
        let find_key = key.as_str();
        self.0
            .iter()
            .find_map(|(key, value)| (key == find_key).then(|| value.to_kv_value()))
    }

    fn count(&self) -> usize {
        self.0.iter().count()
    }
}
