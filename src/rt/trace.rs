//! Tracing utilities.

#![allow(dead_code, unused_imports, missing_docs, missing_debug_implementations)]

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::size_of;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use log::warn;

use crate::rt::ProcessId;

/// Default buffer size of `Log`.
const BUF_SIZE: usize = 128;

/// Trace log.
#[derive(Debug)]
pub(crate) struct Log {
    /// File to write the trace to.
    ///
    /// This file is shared between one or more thread, thus writes to it should
    /// be atomic, e.g. no partial writes. Most OSs support atomic writes up to
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

        let file = OpenOptions::new().append(true).create(true).open(path)?;

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

    /// Append `event` to trace `Log`.
    pub(crate) fn append<E>(&mut self, event: &Metadata<E>) -> io::Result<()>
    where
        E: Event,
    {
        const MAGIC: u32 = 0xC1FC1FB7;

        let stream_count: u32 = self.next_stream_count();
        let start_nanos: u64 = self.nanos_since_epoch(event.start);
        let end_nanos: u64 = self.nanos_since_epoch(event.end);
        let description: &[u8] = E::DESCRIPTION.as_bytes();
        let description_len: u16 = description.len() as u16;

        self.buf.clear();
        self.buf.extend_from_slice(&MAGIC.to_be_bytes());
        self.buf.extend_from_slice(&0u32.to_be_bytes()); // Written later.
        self.buf.extend_from_slice(&self.stream_id.to_be_bytes());
        self.buf.extend_from_slice(&stream_count.to_be_bytes());
        self.buf.extend_from_slice(&start_nanos.to_be_bytes());
        self.buf.extend_from_slice(&end_nanos.to_be_bytes());
        self.buf.extend_from_slice(&description_len.to_be_bytes());
        self.buf.extend_from_slice(description);
        event.event.write_attributes(&mut self.buf);
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
    fn nanos_since_epoch(&self, time: Instant) -> u64 {
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
    const MAGIC: u32 = 0x75D11D4D;
    const PACKET_SIZE: u32 = 23;
    const OPTION_LENGTH: u16 = OPTION.len() as u16;
    const OPTION: &[u8] = b"epoch";

    // Number of nanoseconds since Unix epoch as u64.
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

/// Returns the number of nanoseconds since `epoch`.
///
/// (2 ^ 64) / 1000000000 / (365 * 24 * 60 * 60) ~= 584 years.
/// So restart the application once every 500 years and you're good.
#[track_caller]
fn nanos_since_epoch(epoch: Instant, time: Instant) -> u64 {
    time.duration_since(epoch).as_nanos() as u64
}

/// Start timing for an event (using [`EventTiming`]) if we're tracing, i.e. if
/// `log` is `Some`.
pub(crate) fn start(log: &Option<Log>) -> Option<EventTiming> {
    if log.is_some() {
        Some(EventTiming::start())
    } else {
        None
    }
}

/// Finish a trace, partner function to [`start`].
///
/// If `log` is `Some` `timing` must also be `Some.
#[track_caller]
pub(crate) fn finish<E>(log: &mut Option<Log>, timing: Option<EventTiming>, event: E)
where
    E: Event,
{
    if let Some(log) = log.as_mut() {
        let timing = timing.unwrap();
        let event = timing.finish(event);
        if let Err(err) = log.append(&event) {
            warn!("error writing trace data: {}", err);
        }
    }
}

/// Write the entire `buf`fer into the `output` or return an error.
#[inline(always)]
fn write_once<W>(mut output: W, buf: &[u8]) -> io::Result<()>
where
    W: Write,
{
    output.write(buf).and_then(|written| {
        if written != buf.len() {
            // Not completely correct when going by the name alone, but it's the
            // closest we can get to a descriptive error.
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write entire trace event",
            ))
        } else {
            Ok(())
        }
    })
}

/// Metadata wrapping an [`Event`].
#[derive(Debug)]
pub(crate) struct Metadata<E> {
    start: Instant,
    end: Instant,
    event: E,
}

/// Time an [`Event`].
#[derive(Debug)]
pub(crate) struct EventTiming {
    start: Instant,
}

impl EventTiming {
    /// Start timing.
    pub(crate) fn start() -> EventTiming {
        let start = Instant::now();
        EventTiming { start }
    }

    /// Finish timing `event`.
    pub(crate) fn finish<E>(self, event: E) -> Metadata<E> {
        let end = Instant::now();
        Metadata {
            start: self.start,
            end,
            event,
        }
    }
}

/// Trait that defines an event.
///
/// An event is simple, it has a human-readable description and optional
/// attributes. To create an event type use the [`event!`] macro.
pub(crate) trait Event {
    /// Description of the event in human-readable text, e.g. `run process`.
    const DESCRIPTION: &'static str;

    /// Write attributes related to this event to the `buf`fer.
    ///
    /// For example if we're running a process we would write the process id as
    /// a possible attribute.
    ///
    /// Use the [`WriteAttribute`] trait for this.
    fn write_attributes(&self, buf: &mut Vec<u8>);

    /// Use the [`event!`] macro instead.
    #[doc(hidden)]
    fn _do_not_manually_implement_this_it_wont_work();
}

/// The [`WriteAttribute::TYPE_BYTE`] constants.
const UNSIGNED_INTEGER_BYTE: u8 = 0b001;
const SIGNED_INTEGER_BYTE: u8 = 0b010;
const FLOAT_BYTE: u8 = 0b011;
pub(crate) const STRING_BYTE: u8 = 0b100; // Used in `event!` macro.

// Use in `event!` macro.
pub(crate) trait WriteAttribute {
    /// The type byte for this attribute.
    const TYPE_BYTE: u8;

    /// Must return [`Self::TYPE_BYTE`].
    fn type_byte(&self) -> u8 {
        Self::TYPE_BYTE
    }

    /// Write the contents of the attribute, without type byte.
    fn write_attribute(&self, buf: &mut Vec<u8>);
}

macro_rules! impl_write_attribute {
    ($ty: ty as $f_ty: ty, $type_byte: expr) => {
        impl WriteAttribute for $ty {
            const TYPE_BYTE: u8 = $type_byte;

            fn write_attribute(&self, buf: &mut Vec<u8>) {
                #[allow(trivial_numeric_casts)] // for `u64 as u64`.
                let value = *self as $f_ty;
                buf.extend_from_slice(&value.to_be_bytes());
            }
        }
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

impl WriteAttribute for str {
    const TYPE_BYTE: u8 = STRING_BYTE;

    fn write_attribute(&self, buf: &mut Vec<u8>) {
        let bytes = self.as_bytes();
        let length = bytes.len() as u16;
        buf.extend_from_slice(&length.to_be_bytes());
        buf.extend_from_slice(bytes);
    }
}

impl WriteAttribute for String {
    const TYPE_BYTE: u8 = STRING_BYTE;

    fn write_attribute(&self, buf: &mut Vec<u8>) {
        (&**self).write_attribute(buf)
    }
}

impl<T> WriteAttribute for [T]
where
    T: WriteAttribute,
{
    const TYPE_BYTE: u8 = T::TYPE_BYTE | (1 << 7); // Mark for array.

    fn write_attribute(&self, buf: &mut Vec<u8>) {
        let length = self.len() as u16;
        buf.extend_from_slice(&length.to_be_bytes());

        for attribute in self.iter() {
            attribute.write_attribute(buf)
        }
    }
}

impl<T, const N: usize> WriteAttribute for [T; N]
where
    T: WriteAttribute,
{
    const TYPE_BYTE: u8 = T::TYPE_BYTE | (1 << 7); // Mark for array.

    fn write_attribute(&self, buf: &mut Vec<u8>) {
        (&self[..]).write_attribute(buf)
    }
}

/// Macro to create a new [`Event`] type.
///
/// The attributes supported are currently limited to any sized (un)signed
/// integer (e.g. `u64`, `usize`, `i64` etc), floats (`f32` and `f64`), strings
/// (`str` and `String`) and slices (and arrays).
macro_rules! event {
    ($msg: expr) => {{
        #[allow(missing_docs)]
        struct Event;

        impl $crate::rt::trace::Event for Event {
            const DESCRIPTION: &'static str = $msg;

            fn write_attributes(&self, _: &mut Vec<u8>) { /* Do nothing. */ }

            fn _do_not_manually_implement_this_it_wont_work() {}
        }

        impl std::fmt::Debug for Event {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                use $crate::rt::trace::Event;
                f.debug_struct("Event")
                    .field("description", &Self::DESCRIPTION)
                    .finish()
            }
        }

        Event
    }};
    ($msg: expr, {
        $( $attribute_name: ident : $attribute_type: ty = $attribute_value: expr),+ $(,)*
    }) => {{
        #[allow(missing_docs)]
        struct Event {
            $( $attribute_name : $attribute_type ),+
        }

        impl $crate::rt::trace::Event for Event {
            const DESCRIPTION: &'static str = $msg;

            fn write_attributes(&self, buf: &mut Vec<u8>) {
                use $crate::rt::trace::WriteAttribute;
                $(
                    // Write the attribute name.
                    stringify!($attribute_name).write_attribute(buf);
                    // Write the attribute value.
                    buf.push(self.$attribute_name.type_byte());
                    self.$attribute_name.write_attribute(buf);
                )+
            }

            fn _do_not_manually_implement_this_it_wont_work() {}
        }

        impl std::fmt::Debug for Event {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                use $crate::rt::trace::Event;
                f.debug_struct("Event")
                    .field("description", &Self::DESCRIPTION)
                    $( .field(stringify!($attribute_name), &self.$attribute_name) )+
                    .finish()
            }
        }

        #[allow(clippy::redundant_field_names)]
        Event {
            $( $attribute_name : $attribute_value ),+
        }
    }};
}
