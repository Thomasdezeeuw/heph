//! Tracing utilities.
//!
//! Using the tracing facilities of Heph is a three step process.
//!
//! 1. Enabling tracing.
//! 2. Creating trace events.
//! 3. Interpreting the trace output.
//!
//! # Enabling Tracing
//!
//! Tracing is enabled by calling [`Setup::enable_tracing`] when setting up the
//! runtime.
//!
//! [`Setup::enable_tracing`]: crate::Setup::enable_tracing
//!
//! # Creating Trace Events
//!
//! The runtime already add its own trace events, e.g. when running actors, but
//! users can also log events. Actors can log trace event using the [`Trace`]
//! implementation for their context, i.e. [`actor::Context`] or
//! [`sync::Context`].
//!
//! Calling [`start_trace`] will start timing an event by returning
//! [`EventTiming`]. Next the actor should execute the action(s) it wants to
//! trace, e.g. receiving a message.
//!
//! After the actions have finished [`finish_trace`] should be called with the
//! timing returned by `start_trace`. In addition to the timing it should also
//! include a human readable `description` and optional `attributes`. The
//! attributes are a slice of key-value pairs, where the key is a string and the
//! value is of the type [`AttributeValue`]. [`AttributeValue`] supports most
//! primitives types, compound types are not supported and should be split into
//! multiple key-value pairs.
//!
//! Nested trace events are supported, simply call `start_trace` (and
//! `finish_trace`) twice. In the interpreting of the trace log events are
//! parsed in such a way that parent-child relations become clear if trace
//! events are created by the same actor.
//!
//! [`actor::Context`]: heph::actor::Context
//! [`sync::Context`]: heph::sync::Context
//! [`start_trace`]: Trace::start_trace
//! [`finish_trace`]: Trace::finish_trace
//!
//! ## Notes
//!
//! You might notice that the `start_trace` doesn't actually return
//! `EventTiming`, but `Option<EventTiming>`, and `finish_trace` accepts
//! `Option<EventTiming>`. This is to support the case when tracing is disabled.
//! This makes it easier to leave the code in-place and don't have to deal with
//! `#[cfg(is_tracing_enabled)]` attributes etc. When tracing is disabled
//! `start_trace` will return `None` and if `None` is passed to `finish_trace`
//! it's effectively a no-op.
//!
//! # Interpreting the trace output
//!
//! Once a trace log is created its now time to interpret it. The [Trace Format]
//! design document describes the layout of the trace, found in the `doc`
//! directory of the repository.
//!
//! However as it's a binary format it can be hard to read. So tools are
//! provided to convert into [Chrome's Trace Event Format], which can be viewed
//! using [Catapult]. [Example 8 "Runtime Tracing"] shows a complete example of
//! this.
//!
//! [Trace Format]: https://github.com/Thomasdezeeuw/heph/blob/main/doc/Trace%20Format.md
//! [Chrome's Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview
//! [Catapult]: https://chromium.googlesource.com/catapult/+/refs/heads/master/tracing/README.md
//! [Example 8 "Runtime Tracing"]: https://github.com/Thomasdezeeuw/heph/tree/main/rt/examples#8-runtime-tracing

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{self, AtomicU32};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use log::warn;

/// Default buffer size, only needs to hold a single trace event.
const BUF_SIZE: usize = 128;

/// Stream id used by [`CoordinatorLog`].
const COORDINATOR_STREAM_ID: u32 = 0;
/// Identifier used by the runtime to log events.
const RT_SUBSTREAM_ID: u64 = 0;

/// Trace events.
///
/// See the [`trace`] module for usage.
///
/// [`trace`]: crate::trace
///
/// # Examples
///
/// The following example adds tracing for receiving and handling of a message.
///
/// ```
/// use heph::actor;
/// use heph_rt::ThreadLocal;
/// use heph_rt::trace::Trace;
///
/// async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
///     // Start a trace of receiving and handling a message.
///     let mut trace_timing = ctx.start_trace();
///     while let Ok(msg) = ctx.receive_next().await {
///         // Finish the trace for receiving the message.
///         ctx.finish_trace(trace_timing.clone(), "receiving message", &[("message", &msg)]);
///
///         // Handle the message by printing it.
///         let print_timing = ctx.start_trace();
///         println!("got a message: {msg}");
///
///         // Finish the trace for the printing and handling of the message.
///         ctx.finish_trace(print_timing, "Printing message", &[]);
///         ctx.finish_trace(trace_timing, "Handling message", &[]);
///
///         // Start tracing the next message.
///         trace_timing = ctx.start_trace();
///     }
/// }
///
/// # _ = actor;
/// ```
pub trait Trace {
    /// Start timing an event if tracing is enabled.
    ///
    /// To finish the trace call [`finish_trace`]. See the [`trace`] module for
    /// more information.
    ///
    /// [`finish_trace`]: Trace::finish_trace
    /// [`trace`]: crate::trace
    ///
    /// # Notes
    ///
    /// If [`finish_trace`] is not called no trace event will be written. Be
    /// careful with this when using the [`Try`] (`?`) operator.
    ///
    /// [`Try`]: std::ops::Try
    #[must_use = "tracing events must be finished, otherwise they aren't recorded"]
    fn start_trace(&self) -> Option<EventTiming>;

    /// Finish tracing an event, partner function to [`start_trace`].
    ///
    /// See the [`trace`] module for more information, e.g. what each argument
    /// means.
    ///
    /// [`start_trace`]: Trace::start_trace
    /// [`trace`]: crate::trace
    fn finish_trace(
        &mut self,
        timing: Option<EventTiming>,
        description: &str,
        attributes: &[(&str, &dyn AttributeValue)],
    );
}

/// Trace log for the coordinator.
///
/// From this log more [`Log`]s for the worker threads can be created.
#[derive(Debug)]
pub(crate) struct CoordinatorLog {
    /// Data shared between [`CoordinatorLog`] and mulitple [`Log`]s.
    shared: Arc<SharedLog>,
    /// Used to buffer writes for a single event.
    buf: Vec<u8>,
}

/// Metrics for [`CoordinatorLog`].
#[derive(Debug)]
pub(crate) struct CoordinatorMetrics<'l> {
    pub(crate) file: &'l File,
    pub(crate) counter: u32,
}

impl CoordinatorLog {
    /// Open a new trace log.
    pub(crate) fn open(path: &Path) -> io::Result<CoordinatorLog> {
        // Start with getting the "real" time, using the wall-clock.
        let timestamp = SystemTime::now();
        // Hopefully quickly after get a monotonic time we use as zero-point
        // (i.e. the epoch for this trace).
        let epoch = Instant::now();

        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)?;

        // Write the metadata for the trace log, currently it only sets the
        // epoch time.
        let mut buf = Vec::with_capacity(BUF_SIZE);
        write_epoch_metadata(&mut buf, timestamp);
        write_once(&file, &buf)?;

        Ok(CoordinatorLog {
            shared: Arc::new(SharedLog {
                file,
                counter: AtomicU32::new(0),
                epoch,
            }),
            buf: Vec::with_capacity(BUF_SIZE),
        })
    }

    /// Gather metrics for the coordinator log.
    pub(crate) fn metrics<'l>(&'l self) -> CoordinatorMetrics<'l> {
        CoordinatorMetrics {
            file: &self.shared.file,
            counter: self.shared.counter.load(atomic::Ordering::Relaxed),
        }
    }

    /// Create a new stream with `stream_id`, writing to the same file.
    pub(crate) fn new_stream(&self, stream_id: u32) -> Log {
        Log {
            shared: self.shared.clone(),
            stream_id,
            stream_counter: 0,
            buf: Vec::with_capacity(BUF_SIZE),
        }
    }

    /// Clone the shared log.
    pub(crate) fn clone_shared(&self) -> Arc<SharedLog> {
        self.shared.clone()
    }

    /// Returns the next stream counter.
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn next_stream_count(&mut self) -> u32 {
        // SAFETY: needs to sync with itself.
        self.shared.counter.fetch_add(1, atomic::Ordering::AcqRel)
    }
}

/// Data shared between [`CoordinatorLog`] and mulitple [`Log`]s.
#[derive(Debug)]
pub(crate) struct SharedLog {
    /// File to write the trace to.
    ///
    /// This file is shared between one or more threads, thus writes to it
    /// should be atomic, i.e. no partial writes. Most OSs support atomic writes
    /// up to a page size (usually 4KB).
    file: File,
    /// Counter for the stream with id 0, which is owned by the coordinator, but
    /// also used by the worker threads for thread-safe actors.
    counter: AtomicU32,
    /// Time which we use as zero, or epoch, time for all events.
    epoch: Instant,
}

/// Trace log.
#[derive(Debug)]
pub(crate) struct Log {
    /// Data shared between [`CoordinatorLog`] and multiple [`Log`]s.
    shared: Arc<SharedLog>,
    /// Id of the stream, used in writing events.
    /// **Immutable**.
    stream_id: u32,
    /// Count for the events we're writing to this stream.
    stream_counter: u32,
    /// Used to buffer writes for a single event.
    buf: Vec<u8>,
}

/// Metrics for [`Log`].
#[derive(Debug)]
pub(crate) struct Metrics {
    pub(crate) counter: u32,
}

impl Log {
    /// Returns the next stream counter.
    fn next_stream_count(&mut self) -> u32 {
        let count = self.stream_counter;
        self.stream_counter = self.stream_counter.wrapping_add(1);
        count
    }

    /// Gather metrics for the log.
    pub(crate) fn metrics(&self) -> Metrics {
        Metrics {
            counter: self.shared.counter.load(atomic::Ordering::Relaxed),
        }
    }
}

/// Write an epoch metadata packet to `buf`.
fn write_epoch_metadata(buf: &mut Vec<u8>, time: SystemTime) {
    #[allow(clippy::unreadable_literal)]
    const MAGIC: u32 = 0x75D11D4D;
    const PACKET_SIZE: u32 = 23;
    // SAFETY: `OPTION` is small enough to fit it's length in `u16`.
    #[allow(clippy::cast_possible_truncation)]
    const OPTION_LENGTH: u16 = OPTION.len() as u16;
    const OPTION: &[u8] = b"epoch";

    // Number of nanoseconds since Unix epoch as u64.
    // SAFETY: this overflows in the year 2500+, so this will be good for a
    // while.
    #[allow(clippy::cast_possible_truncation)]
    let nanos_since_unix = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    buf.extend_from_slice(&MAGIC.to_be_bytes());
    buf.extend_from_slice(&PACKET_SIZE.to_be_bytes());
    buf.extend_from_slice(&OPTION_LENGTH.to_be_bytes());
    buf.extend_from_slice(OPTION);
    buf.extend_from_slice(&nanos_since_unix.to_be_bytes());
}

/// Write the entire `buf`fer into the `output` or return an error.
fn write_once<W>(mut output: W, buf: &[u8]) -> io::Result<()>
where
    W: Write,
{
    output.write(buf).and_then(|written| {
        if written == buf.len() {
            Ok(())
        } else {
            // Not completely correct when going by the name alone, but it's the
            // closest we can get to a descriptive error.
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write entire trace event",
            ))
        }
    })
}

impl Clone for Log {
    fn clone(&self) -> Log {
        Log {
            shared: self.shared.clone(),
            stream_id: self.stream_id,
            stream_counter: 0,
            buf: Vec::with_capacity(BUF_SIZE),
        }
    }
}

/// Start timing an event (using [`EventTiming`]) if we're tracing, i.e. if
/// `log` is `Some`.
pub(crate) fn start<L>(log: &Option<L>) -> Option<EventTiming>
where
    L: TraceLog,
{
    log.is_some().then(EventTiming::start)
}

/// Trait to call [`finish`] on both [`CoordinatorLog`] and [`Log`].
pub(crate) trait TraceLog {
    /// Append a new `event` to the log.
    fn append(&mut self, substream_id: u64, event: &Event<'_>) -> io::Result<()>;
}

impl<L> TraceLog for &'_ mut L
where
    L: TraceLog,
{
    fn append(&mut self, substream_id: u64, event: &Event<'_>) -> io::Result<()> {
        L::append(self, substream_id, event)
    }
}

impl TraceLog for CoordinatorLog {
    fn append(&mut self, substream_id: u64, event: &Event<'_>) -> io::Result<()> {
        let stream_count = self.next_stream_count();
        format_event(
            &mut self.buf,
            self.shared.epoch,
            COORDINATOR_STREAM_ID,
            stream_count,
            substream_id,
            event,
        );
        // TODO: buffer events? If buf.len() + packet_size >= 4k -> write first?
        write_once(&self.shared.file, &self.buf)
    }
}

impl TraceLog for Log {
    fn append(&mut self, substream_id: u64, event: &Event<'_>) -> io::Result<()> {
        let stream_count = self.next_stream_count();
        format_event(
            &mut self.buf,
            self.shared.epoch,
            self.stream_id,
            stream_count,
            substream_id,
            event,
        );
        // TODO: buffer events? If buf.len() + packet_size >= 4k -> write first?
        write_once(&self.shared.file, &self.buf)
    }
}

/// # Notes
///
/// Uses stream id 0.
///
/// This uses thread-local storage, prefer to use the [`CoordinatorLog`] or
/// [`Log`] implementations.
impl<'a> TraceLog for &'a SharedLog {
    fn append(&mut self, substream_id: u64, event: &Event<'_>) -> io::Result<()> {
        thread_local! {
            static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        }

        BUF.with(|buf| {
            let mut buf = buf.borrow_mut();
            // SAFETY: needs to sync with itself.
            let stream_count = self.counter.fetch_add(1, atomic::Ordering::AcqRel);
            format_event(
                &mut buf,
                self.epoch,
                COORDINATOR_STREAM_ID,
                stream_count,
                substream_id,
                event,
            );
            // TODO: buffer events? If buf.len() + packet_size >= 4k -> write first?
            write_once(&self.file, &buf)
        })
    }
}

/// Format the `event` writing to `buf`fer.
fn format_event(
    buf: &mut Vec<u8>,
    epoch: Instant,
    stream_id: u32,
    stream_count: u32,
    substream_id: u64,
    event: &Event<'_>,
) {
    #[allow(clippy::unreadable_literal)]
    const MAGIC: u32 = 0xC1FC1FB7;

    let start_nanos: u64 = nanos_since_epoch(epoch, event.start);
    let end_nanos: u64 = nanos_since_epoch(epoch, event.end);
    let description: &[u8] = event.description.as_bytes();
    // SAFETY: length has a debug_assert in `finish`.
    #[allow(clippy::cast_possible_truncation)]
    let description_len: u16 = description.len() as u16;

    buf.clear();
    buf.extend_from_slice(&MAGIC.to_be_bytes());
    buf.extend_from_slice(&0_u32.to_be_bytes()); // Written later.
    buf.extend_from_slice(&stream_id.to_be_bytes());
    buf.extend_from_slice(&stream_count.to_be_bytes());
    buf.extend_from_slice(&substream_id.to_be_bytes());
    buf.extend_from_slice(&start_nanos.to_be_bytes());
    buf.extend_from_slice(&end_nanos.to_be_bytes());
    buf.extend_from_slice(&description_len.to_be_bytes());
    buf.extend_from_slice(description);
    for (name, value) in event.attributes {
        use private::AttributeValue;
        (**name).write_attribute(buf);
        buf.push(value.type_byte());
        value.write_attribute(buf);
    }
    // TODO: check maximum packet length.
    #[allow(clippy::cast_possible_truncation)]
    let packet_size = buf.len() as u32;
    buf[4..8].copy_from_slice(&packet_size.to_be_bytes());
}

/// Returns the number of nanoseconds since the trace's epoch.
///
/// (2 ^ 64) / 1000000000 / (365 * 24 * 60 * 60) ~= 584 years.
/// So restart the application once every 500 years and you're good.
#[track_caller]
#[allow(clippy::cast_possible_truncation)]
fn nanos_since_epoch(epoch: Instant, time: Instant) -> u64 {
    // SAFETY: this overflows after 500+ years as per the function doc.
    time.duration_since(epoch).as_nanos() as u64
}

/// Finish tracing an event, partner function to [`start`].
///
/// If `log` or `timing` is `None` this does nothing.
pub(crate) fn finish<L>(
    log: Option<L>,
    timing: Option<EventTiming>,
    substream_id: u64,
    description: &str,
    attributes: &[(&str, &dyn AttributeValue)],
) where
    L: TraceLog,
{
    debug_assert!(
        description.len() < u16::MAX as usize,
        "description for trace event too long"
    );
    if let (Some(mut log), Some(timing)) = (log, timing) {
        let event = timing.finish(description, attributes);
        if let Err(err) = log.append(substream_id, &event) {
            warn!("error writing trace data: {err}");
        }
    }
}

/// [`finish`] for the runtime.
pub(crate) fn finish_rt<L>(
    log: Option<L>,
    timing: Option<EventTiming>,
    description: &str,
    attributes: &[(&str, &dyn AttributeValue)],
) where
    L: TraceLog,
{
    finish(log, timing, RT_SUBSTREAM_ID, description, attributes);
}

/// Timing an event.
#[derive(Clone, Debug)]
#[must_use = "tracing events must be finished, otherwise they aren't recorded"]
pub struct EventTiming {
    start: Instant,
}

impl EventTiming {
    /// Start timing an event.
    fn start() -> EventTiming {
        let start = Instant::now();
        EventTiming { start }
    }

    /// Finish timing an event.
    fn finish<'e>(
        self,
        description: &'e str,
        attributes: &'e [(&'e str, &'e dyn AttributeValue)],
    ) -> Event<'e> {
        Event {
            start: self.start,
            end: Instant::now(),
            description,
            attributes,
        }
    }
}

/// A trace event.
pub(crate) struct Event<'e> {
    start: Instant,
    end: Instant,
    description: &'e str,
    attributes: &'e [(&'e str, &'e dyn AttributeValue)],
}

/// The `AttributeValue` trait defines what kind of types are supported as
/// attribute values in tracing.
///
/// This trait is private and is implemented for only a limited number of types,
/// specifically:
/// * Unsigned integers, i.e. `u8`, `u16`, etc.
/// * Signed integers, i.e. `i8`, `i16`, etc.
/// * Floating point numbers, i.e. `f32` and `f64`.
/// * Strings, i.e. `&str` and `String`.
/// * Array or slice of one of the types above.
pub trait AttributeValue: private::AttributeValue {}

impl<'a, T> AttributeValue for &'a T where T: AttributeValue + ?Sized {}

mod private {
    //! Module with private version of [`AttributeValue`].

    use std::num::{
        NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroIsize, NonZeroU16, NonZeroU32,
        NonZeroU64, NonZeroU8, NonZeroUsize,
    };

    /// The [`AttributeValue::type_byte`] constants.
    const UNSIGNED_INTEGER_BYTE: u8 = 0b001;
    const SIGNED_INTEGER_BYTE: u8 = 0b010;
    const FLOAT_BYTE: u8 = 0b011;
    const STRING_BYTE: u8 = 0b100;
    /// Marks a type bytes as array.
    const ARRAY_MARKER_BYTE: u8 = 1 << 7;

    /// Trait that defines how to write an attribute value.
    pub trait AttributeValue {
        /// The type byte for this attribute value.
        // NOTE: this should be a associated constant, however that is not
        // object safe.
        fn type_byte(&self) -> u8;

        /// Write the contents of the attribute, without type byte.
        fn write_attribute(&self, buf: &mut Vec<u8>);
    }

    impl<'a, T> AttributeValue for &'a T
    where
        T: AttributeValue + ?Sized,
    {
        fn type_byte(&self) -> u8 {
            (**self).type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            (**self).write_attribute(buf);
        }
    }

    macro_rules! impl_write_attribute {
        ($ty: ty as $f_ty: ty, $n_ty: ty, $type_byte: expr) => {
            impl_write_attribute!($ty as $f_ty, $type_byte);

            impl AttributeValue for $n_ty {
                fn type_byte(&self) -> u8 {
                    $type_byte
                }

                fn write_attribute(&self, buf: &mut Vec<u8>) {
                    #[allow(trivial_numeric_casts, clippy::cast_lossless)] // for `u64 as u64`, etc.
                    let value = self.get() as $f_ty;
                    buf.extend_from_slice(&value.to_be_bytes());
                }
            }

            impl super::AttributeValue for $n_ty {}
        };
        ($ty: ty as $f_ty: ty, $type_byte: expr) => {
            impl AttributeValue for $ty {
                fn type_byte(&self) -> u8 {
                    $type_byte
                }

                fn write_attribute(&self, buf: &mut Vec<u8>) {
                    #[allow(trivial_numeric_casts, clippy::cast_lossless)] // for `u64 as u64`, etc.
                    let value = *self as $f_ty;
                    buf.extend_from_slice(&value.to_be_bytes());
                }
            }

            impl super::AttributeValue for $ty {}
        };
    }

    impl_write_attribute!(u8 as u64, NonZeroU8, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u16 as u64, NonZeroU16, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u32 as u64, NonZeroU32, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u64 as u64, NonZeroU64, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(usize as u64, NonZeroUsize, UNSIGNED_INTEGER_BYTE);

    impl_write_attribute!(i8 as i64, NonZeroI8, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i16 as i64, NonZeroI16, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i32 as i64, NonZeroI32, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i64 as i64, NonZeroI64, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(isize as i64, NonZeroIsize, SIGNED_INTEGER_BYTE);

    // Floats don't have non-zero variants.
    impl_write_attribute!(f32 as f64, FLOAT_BYTE);
    impl_write_attribute!(f64 as f64, FLOAT_BYTE);

    impl AttributeValue for str {
        fn type_byte(&self) -> u8 {
            STRING_BYTE
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            let bytes = self.as_bytes();
            debug_assert!(bytes.len() < u16::MAX as usize);
            #[allow(clippy::cast_possible_truncation)]
            let length = bytes.len() as u16;
            buf.extend_from_slice(&length.to_be_bytes());
            buf.extend_from_slice(bytes);
        }
    }

    impl super::AttributeValue for str {}

    impl AttributeValue for String {
        fn type_byte(&self) -> u8 {
            (**self).type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            (**self).write_attribute(buf);
        }
    }

    impl super::AttributeValue for String {}

    impl<T> AttributeValue for [T]
    where
        T: AttributeValue + Default,
    {
        fn type_byte(&self) -> u8 {
            let type_byte = match self.first() {
                Some(elem) => elem.type_byte(),
                // NOTE: this is not ideal...
                None => T::default().type_byte(),
            };
            type_byte | ARRAY_MARKER_BYTE
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            debug_assert!(self.len() < u16::MAX as usize);
            #[allow(clippy::cast_possible_truncation)]
            let length = self.len() as u16;
            buf.extend_from_slice(&length.to_be_bytes());
            for attribute in self {
                attribute.write_attribute(buf);
            }
        }
    }

    impl<T> super::AttributeValue for [T] where T: super::AttributeValue + Default {}

    impl<T, const N: usize> AttributeValue for [T; N]
    where
        T: AttributeValue + Default,
    {
        fn type_byte(&self) -> u8 {
            self[..].type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            self[..].write_attribute(buf);
        }
    }

    impl<T, const N: usize> super::AttributeValue for [T; N] where T: super::AttributeValue + Default {}
}
