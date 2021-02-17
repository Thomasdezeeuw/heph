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
//! [`Setup::enable_tracing`]: crate::rt::Setup::enable_tracing
//!
//! # Creating Trace Events
//!
//! The runtime already add its own trace events, e.g. when running actors, but
//! users can also log events. Actors can log trace event using the
//! `start_trace` and `finish_trace` methods on their context, i.e.
//! [`actor::Context`] or [`SyncContext`].
//!
//! Calling `start_trace` will start timing an event by returning
//! [`EventTiming`]. Next the actor should execute the action(s) it wants to
//! trace, e.g. receiving a message.
//!
//! After the actions have finished `finish_trace` should be called with the
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
//! [`actor::Context`]: crate::actor::Context
//! [`SyncContext`]: crate::actor::sync::SyncContext
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
//! [Trace Format]: https://github.com/Thomasdezeeuw/heph/blob/master/doc/Trace%20Format.md
//! [Chrome's Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview
//! [Catapult]: https://chromium.googlesource.com/catapult/+/refs/heads/master/tracing/README.md
//! [Example 8 "Runtime Tracing"]: https://github.com/Thomasdezeeuw/heph/blob/master/examples/README.md#8-runtime-tracing

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Instant, SystemTime};

use log::warn;

/// Default buffer size, only needs to hold a single trace event.
const BUF_SIZE: usize = 128;

/// Trace log.
#[derive(Debug)]
pub(crate) struct Log {
    /// File to write the trace to.
    ///
    /// This file is shared between one or more thread, thus writes to it should
    /// be atomic, i.e. no partial writes. Most OSs support atomic writes up to
    /// a page size (usally 4kB).
    file: File,
    /// Used to buffer writes for a single event.
    buf: Vec<u8>,
    /// Id of the stream, used in writing events.
    stream_id: u32,
    /// Count for the events we're writing to this stream.
    stream_counter: u32,
    /// Time which we use as zero, or epoch, time for all events.
    epoch: Instant,
}

impl Log {
    /// Open a new trace `Log`.
    pub(super) fn open(path: &Path, stream_id: u32) -> io::Result<Log> {
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

        Ok(Log {
            file,
            stream_id,
            stream_counter: 0,
            buf,
            epoch,
        })
    }

    /// Create a new stream with `stream_id`, writing to the same (duplicated)
    /// file.
    pub(super) fn new_stream(&self, stream_id: u32) -> io::Result<Log> {
        self.file.try_clone().map(|file| Log {
            file,
            stream_id,
            stream_counter: 0,
            buf: Vec::with_capacity(BUF_SIZE),
            epoch: self.epoch,
        })
    }

    /// Attempt to clone the log, writing the same stream.
    pub(super) fn try_clone(&self) -> io::Result<Log> {
        self.file.try_clone().map(|file| Log {
            file,
            stream_id: self.stream_id,
            stream_counter: 0,
            buf: Vec::with_capacity(BUF_SIZE),
            epoch: self.epoch,
        })
    }

    /// Append `event` to trace `Log`.
    fn append(&mut self, event: &Event<'_>) -> io::Result<()> {
        #[allow(clippy::unreadable_literal)]
        const MAGIC: u32 = 0xC1FC1FB7;

        let stream_count: u32 = self.next_stream_count();
        let start_nanos: u64 = self.nanos_since_epoch(event.start);
        let end_nanos: u64 = self.nanos_since_epoch(event.end);
        let description: &[u8] = event.description.as_bytes();
        // Safety: length has a debug_assert in `finish`.
        #[allow(clippy::cast_possible_truncation)]
        let description_len: u16 = description.len() as u16;

        self.buf.clear();
        self.buf.extend_from_slice(&MAGIC.to_be_bytes());
        self.buf.extend_from_slice(&0_u32.to_be_bytes()); // Written later.
        self.buf.extend_from_slice(&self.stream_id.to_be_bytes());
        self.buf.extend_from_slice(&stream_count.to_be_bytes());
        self.buf.extend_from_slice(&start_nanos.to_be_bytes());
        self.buf.extend_from_slice(&end_nanos.to_be_bytes());
        self.buf.extend_from_slice(&description_len.to_be_bytes());
        self.buf.extend_from_slice(description);
        for (name, value) in event.attributes {
            use private::AttributeValue;
            (&**name).write_attribute(&mut self.buf);
            self.buf.push(value.type_byte());
            value.write_attribute(&mut self.buf);
        }
        // TODO: check maximum packet length.
        #[allow(clippy::cast_possible_truncation)]
        let packet_size = self.buf.len() as u32;
        self.buf[4..8].copy_from_slice(&packet_size.to_be_bytes());

        // TODO: buffer events? If buf.len() + packet_size >= 4k -> write first?
        write_once(&self.file, &self.buf)
    }

    /// Returns the number of nanoseconds since the trace's epoch.
    ///
    /// (2 ^ 64) / 1000000000 / (365 * 24 * 60 * 60) ~= 584 years.
    /// So restart the application once every 500 years and you're good.
    #[track_caller]
    #[allow(clippy::cast_possible_truncation)]
    fn nanos_since_epoch(&self, time: Instant) -> u64 {
        // Safety: this overflows after 500+ years as per the function doc.
        time.duration_since(self.epoch).as_nanos() as u64
    }

    /// Returns the next stream counter.
    fn next_stream_count(&mut self) -> u32 {
        let count = self.stream_counter;
        self.stream_counter = self.stream_counter.wrapping_add(1);
        count
    }
}

/// Write an epoch metadata packet to `buf`.
fn write_epoch_metadata(buf: &mut Vec<u8>, time: SystemTime) {
    #[allow(clippy::unreadable_literal)]
    const MAGIC: u32 = 0x75D11D4D;
    const PACKET_SIZE: u32 = 23;
    // Safety: `OPTION` is small enough to fit it's length in `u16`.
    #[allow(clippy::cast_possible_truncation)]
    const OPTION_LENGTH: u16 = OPTION.len() as u16;
    const OPTION: &[u8] = b"epoch";

    // Number of nanoseconds since Unix epoch as u64.
    // Safety: this overflows in the year 2500+, so this will be good for a
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
#[inline(always)]
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

/// Start timing an event (using [`EventTiming`]) if we're tracing, i.e. if
/// `log` is `Some`.
pub(crate) fn start(log: &Option<Log>) -> Option<EventTiming> {
    if log.is_some() {
        Some(EventTiming::start())
    } else {
        None
    }
}

/// Finish tracing an event, partner function to [`start`].
///
/// If `log` or `timing` is `None` this does nothing.
pub(crate) fn finish(
    log: &mut Option<Log>,
    timing: Option<EventTiming>,
    description: &str,
    attributes: &[(&str, &dyn AttributeValue)],
) {
    debug_assert!(
        description.len() < u16::MAX as usize,
        "description for trace event too long"
    );
    if let (Some(log), Some(timing)) = (log, timing) {
        let event = timing.finish(description, attributes);
        if let Err(err) = log.append(&event) {
            warn!("error writing trace data: {}", err);
        }
    }
}

/// Timing an event.
#[derive(Debug)]
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
        let end = Instant::now();
        Event {
            start: self.start,
            end,
            description,
            attributes,
        }
    }
}

/// A trace event.
struct Event<'e> {
    start: Instant,
    end: Instant,
    description: &'e str,
    attributes: &'e [(&'e str, &'e dyn AttributeValue)],
}

/// The `AttributeValue` trait defines what kind of types are supported as
/// attribute values in tracing.
///
/// This trait is private and is implemented for only a limited number of types,
/// specficically:
/// * Unsigned integers, i.e. `u8`, `u16`, etc.
/// * Signed integers, i.e. `i8`, `i16`, etc.
/// * Floating point numbers, i.e. `f32` and `f64`.
/// * Strings, i.e. `&str` and `String`.
/// * Array or slice of one of the types above.
pub trait AttributeValue: private::AttributeValue {}

impl<'a, T> AttributeValue for &'a T where T: AttributeValue + ?Sized {}

mod private {
    //! Module with private version of [`AttributeValue`].

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
        // NOTE: this should be a assiociated constant, however that is not
        // object safe.
        fn type_byte(&self) -> u8;

        /// Write the contents of the attribute, without type byte.
        fn write_attribute(&self, buf: &mut Vec<u8>);
    }

    impl<'a, T> AttributeValue for &'a T
    where
        T: AttributeValue + ?Sized,
    {
        #[inline(always)]
        fn type_byte(&self) -> u8 {
            (&**self).type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            (&**self).write_attribute(buf)
        }
    }

    macro_rules! impl_write_attribute {
        ($ty: ty as $f_ty: ty, $type_byte: expr) => {
            impl AttributeValue for $ty {
                #[inline(always)]
                fn type_byte(&self) -> u8 {
                    $type_byte
                }

                fn write_attribute(&self, buf: &mut Vec<u8>) {
                    #[allow(trivial_numeric_casts)] // for `u64 as u64`, etc.
                    let value = *self as $f_ty;
                    buf.extend_from_slice(&value.to_be_bytes());
                }
            }

            impl super::AttributeValue for $ty {}
        };
    }

    impl_write_attribute!(u8 as u64, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u16 as u64, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u32 as u64, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(u64 as u64, UNSIGNED_INTEGER_BYTE);
    impl_write_attribute!(usize as u64, UNSIGNED_INTEGER_BYTE);

    impl_write_attribute!(i8 as i64, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i16 as i64, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i32 as i64, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(i64 as i64, SIGNED_INTEGER_BYTE);
    impl_write_attribute!(isize as i64, SIGNED_INTEGER_BYTE);

    impl_write_attribute!(f32 as f64, FLOAT_BYTE);
    impl_write_attribute!(f64 as f64, FLOAT_BYTE);

    impl AttributeValue for str {
        #[inline(always)]
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
        #[inline(always)]
        fn type_byte(&self) -> u8 {
            (&**self).type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            (&**self).write_attribute(buf)
        }
    }

    impl super::AttributeValue for String {}

    impl<T> AttributeValue for [T]
    where
        T: AttributeValue + Default,
    {
        #[inline(always)]
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
            for attribute in self.iter() {
                attribute.write_attribute(buf)
            }
        }
    }

    impl<T> super::AttributeValue for [T] where T: super::AttributeValue + Default {}

    impl<T, const N: usize> AttributeValue for [T; N]
    where
        T: AttributeValue + Default,
    {
        #[inline(always)]
        fn type_byte(&self) -> u8 {
            (&self[..]).type_byte()
        }

        fn write_attribute(&self, buf: &mut Vec<u8>) {
            (&self[..]).write_attribute(buf)
        }
    }

    impl<T, const N: usize> super::AttributeValue for [T; N] where T: super::AttributeValue + Default {}
}
