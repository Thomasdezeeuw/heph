//! Metric collection.
//!
//! Various types support metric collection. What metrics are collected and how
//! they look is different per type. To see what types support metric collection
//! see the [implementors of Metrics].
//!
//! This module has two main exports.
//!
//! First, the [`Metrics`] trait. It allows you to access the collected metrics.
//! It returns an iterator that iterates over the metrics in metric name-value
//! pairs.
//!
//! Second, the [`Metric`] type, which is the container type for all metrics.
//!
//! [implementors of Metrics]: Metrics#implementors

use std::sync::atomic::{AtomicUsize, Ordering};

/// Access to metrics.
pub trait Metrics {
    /// Container type for the metrics that are collected. The container must be
    /// an iterator that iterates over the metric name-value pairs.
    type Metrics: Iterator<Item = (&'static str, Metric)>;

    /// Get all the metrics from this type.
    fn metrics(&self) -> Self::Metrics;

    /// Get the metric with `name` from this type, if any
    fn metric(&self, name: &str) -> Option<Metric> {
        self.metrics()
            .find_map(|(n, metric)| (n == name).then_some(metric))
    }
}

/// Single metric.
///
/// Type that can hold different types of metrics.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Metric {
    /// Simple counter.
    ///
    /// A counter represent a single monotonic value, which means the value can
    /// only be incremented, not decremented. Only after a restart may it be
    /// reset to zero.
    ///
    /// Examples of counters are the amount of bytes send or received on a
    /// connection.
    Counter(usize),
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
        let _ = self.0.fetch_add(n, Ordering::AcqRel);
    }

    /// Get the current value of the counter.
    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

impl From<&AtomicCounter> for Metric {
    fn from(counter: &AtomicCounter) -> Metric {
        Metric::Counter(counter.get())
    }
}

/// Macro to create metrics structure for a given type.
macro_rules! create_metric {
    (
        $vis: vis struct $name: ident for $for_ty: ident $( < $( $generic: ident )+ > )? {
            // `$field_doc` documents the metric as field doc and in the structure docs, max ~1 line.
            // `$field` is used as name for `Metrics` implementation.
            // `$field_ty` must support `Metric::from(&$value)`.
            // `$metric_ty` must be a variant of `Metric`.
            $(
            $(#[ $field_doc: meta ])+
            $field: ident : $field_ty: ident -> $metric_ty: ident,
            )+
        }
    ) => {
        #[doc = concat!("Metrics for [`", stringify!($for_ty), "`].")]
        #[derive(Debug)]
        $vis struct $name {
            $(
            $(#[ $field_doc ])+
            $vis $field: $crate::metrics::$field_ty,
            )*
        }

        impl $name {
            /// Create empty metrics.
            const fn empty() -> $name {
                $name {
                    $( $field: $crate::metrics::$field_ty::new() ),*
                }
            }
        }

        /// Collects the following metrics:
        $(
        #[doc = concat!(" * `", stringify!($field), "`: ")]
        $(#[ $field_doc ])+
        #[doc = concat!("Type [`", stringify!($metric_ty), "`](crate::metrics::Metric::", stringify!($metric_ty), ").")]
        )*
        impl$( < $( $generic )+ > )? $crate::metrics::Metrics for $for_ty$( < $( $generic )+ > )? {
            type Metrics = impl Iterator<Item = (&'static str, crate::metrics::Metric)> + std::iter::ExactSizeIterator + std::iter::FusedIterator;

            fn metrics(&self) -> Self::Metrics {
                std::iter::IntoIterator::into_iter([
                    $( (stringify!($field), crate::metrics::Metric::from(&self.metrics.$field)) ),*
                ])
            }
        }
    };
}

pub(crate) use create_metric;
