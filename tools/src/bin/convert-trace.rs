//! Tool to convert a Heph trace to [Chrome's Trace Event Format] so it can be
//! opened by [Catapult trace view].
//!
//! [Chrome's Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview
//! [Catapult trace view]: https://chromium.googlesource.com/catapult/+/refs/heads/master/tracing/README.md

use std::collections::hash_map::{Entry, HashMap};
use std::convert::TryInto;
use std::env::args;
use std::fs::{File, OpenOptions};
use std::io::{self, stdin, Read, Stdin, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{fmt, str};

fn main() {
    let mut args = args().skip(1);
    let input = args.next().expect("missing input trace file path");
    let output = if let Some(output) = args.next() {
        PathBuf::from(output)
    } else {
        let end_idx = input.rfind('.').unwrap_or(input.len());
        let mut output = PathBuf::from(&input[..end_idx]);
        // If the input has a single extension this will add `json` to it.
        // If however it has two extensions, e.g. `.bin.log` this will
        // overwrite the extension.
        output.set_extension("json");
        output
    };

    let mut trace = Trace::open(input).expect("can't open trace file");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .expect("can't open output file");

    output
        .write_all(b"{\n\t\"displayTimeUnit\": \"ns\",\n\t\"traceEvents\": [\n")
        .expect("failed to write header to output");

    // Sometimes `Instant` returns a value that is equal to a previously
    // returned value. Catapult can't really deal with this and creates two
    // overlapping events, making them both unreadable and unusable.
    // To fix this change the starting time and duration by a few microseconds
    // to ensure the two events don't start at the same time.
    //
    // Maps `(pid, tid)` -> `timestamp` -> `duration`.
    let mut times: HashMap<(u32, u64), HashMap<u128, u128>> = HashMap::new();

    let mut first = true;
    for event in trace.events() {
        let event = event.expect("error reading trace file");

        let mut timestamp = event
            .start
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros();
        let mut duration = event.end.duration_since(event.start).unwrap().as_micros();

        let process_id = event.stream_id;
        let thread_id = event.substream_id;
        loop {
            let key = (process_id, thread_id);
            match times.entry(key).or_default().entry(timestamp) {
                Entry::Vacant(entry) => {
                    entry.insert(duration);
                    break;
                }
                Entry::Occupied(entry) => {
                    let other_duration = *entry.get();
                    if other_duration > duration {
                        // Other event is the *overlapping* event. Delay the
                        // start of this event and decrease it's duration.
                        timestamp += 1;
                        duration -= 1;
                    } else {
                        // This is the *overlapping* event, grow it.
                        timestamp -= 1;
                        duration += 1;
                    }
                }
            }
        }

        write!(
            output,
            "{}\t\t{{\"pid\": {process_id}, \"tid\": {thread_id}, \"ts\": {timestamp}, \"dur\": {duration}, \"name\": \"{}\"",
            if first { "" } else { ",\n" },
            event.description,
        )
        .expect("failed to write event to output");
        first = false;
        let mut first_attribute = true;
        if !event.attributes.is_empty() {
            output
                .write_all(b", \"args\": {")
                .expect("failed to write event to output");
            for (name, value) in &event.attributes {
                let fmt_args = match value {
                    // NOTE: `format_args!` is useless.
                    Value::Unsigned(value) => format!("\"{name}\": {value}"),
                    Value::Signed(value) => format!("\"{name}\": {value}"),
                    Value::Float(value) => format!("\"{name}\": {value}"),
                    Value::String(value) => format!("\"{name}\": \"{value}\""),
                };
                write!(
                    output,
                    "{}{fmt_args}",
                    if first_attribute { "" } else { ", " },
                )
                .expect("failed to write event to output");
                first_attribute = false;
            }
            output
                .write_all(b"}")
                .expect("failed to write event to output");
        }
        output
            .write_all(b", \"ph\": \"X\", \"cat\": \"\"}")
            .expect("failed to write event to output");
    }

    output
        .write_all(b"\n\t]\n}")
        .expect("failed to write footer to output");

    println!("OK.");
}

pub struct Trace<R> {
    reader: R,
    epoch: SystemTime,
    // TODO: use VecDeque?
    buf: Vec<u8>,
}

impl Trace<File> {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Trace<File>> {
        File::open(path).map(Trace::from_reader)
    }
}

impl Trace<Stdin> {
    pub fn from_stdin() -> Trace<Stdin> {
        Trace::from_reader(stdin())
    }
}

impl<R> Trace<R> {
    pub fn from_reader(reader: R) -> Trace<R> {
        Trace {
            reader,
            epoch: SystemTime::now(),
            buf: Vec::with_capacity(4096),
        }
    }

    pub fn events<'t>(&'t mut self) -> TraceEvents<'t, R> {
        TraceEvents { trace: self }
    }
}

impl<R> Trace<R>
where
    R: Read,
{
    fn fill_buffer(&mut self) -> io::Result<()> {
        let original_length = self.buf.len();
        self.buf.resize(self.buf.capacity(), 0);
        match self.reader.read(&mut self.buf[original_length..]) {
            // TODO: handle 0 bytes read?
            Ok(n) => {
                self.buf.truncate(original_length + n);
                Ok(())
            }
            Err(err) => {
                self.buf.truncate(original_length);
                Err(err)
            }
        }
    }
}

// TODO: when hitting error maybe seek until the next magic value and continue
// from there?
pub struct TraceEvents<'t, R> {
    trace: &'t mut Trace<R>,
}

#[allow(clippy::unreadable_literal)]
const METADATA_MAGIC: u32 = 0x75D11D4D;
#[allow(clippy::unreadable_literal)]
const EVENT_MAGIC: u32 = 0xC1FC1FB7;

/// Minimum amount of bytes in the buffer before we read again.
const MIN_BUF_SIZE: usize = 128;

// TODO: use `cmp::min`, once that stable as constant.
const MIN_PACKET_SIZE: usize = if MIN_METADATA_PACKET_SIZE < MIN_EVENT_PACKET_SIZE {
    MIN_METADATA_PACKET_SIZE
} else {
    MIN_EVENT_PACKET_SIZE
};
const MIN_METADATA_PACKET_SIZE: usize = 10;
const MIN_EVENT_PACKET_SIZE: usize = 34;

impl<'t, R> Iterator for TraceEvents<'t, R>
where
    R: Read,
{
    type Item = Result<Event, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        let trace = &mut *self.trace;
        if trace.buf.len() < MIN_BUF_SIZE {
            if let Err(err) = trace.fill_buffer() {
                return Some(Err(ParseError::IO(err)));
            }
        }

        // Ensure we can read at least one packet.
        if trace.buf.is_empty() {
            return None;
        } else if trace.buf.len() < MIN_PACKET_SIZE {
            return Some(Err(ParseError::MissingPacketData {
                got: trace.buf.len(),
                want: MIN_PACKET_SIZE,
            }));
        }

        let (_, magic) = parse_u32(&trace.buf);
        match magic {
            METADATA_MAGIC => {
                if let Err(err) = self.apply_metadata_packet() {
                    Some(Err(err))
                } else {
                    self.next()
                }
            }
            EVENT_MAGIC => Some(self.parse_event_packet()),
            magic => Some(Err(ParseError::InvalidMagic(magic))),
        }
    }
}

impl<'t, R> TraceEvents<'t, R>
where
    R: Read,
{
    fn apply_metadata_packet(&mut self) -> Result<(), ParseError> {
        let trace = &mut *self.trace;
        debug_assert_eq!(trace.buf[0..4], METADATA_MAGIC.to_be_bytes());

        let (left, packet_size) = parse_u32(&trace.buf[4..]);
        let packet_size = packet_size as usize;
        if packet_size <= MIN_METADATA_PACKET_SIZE {
            return Err(ParseError::PacketTooSmall {
                packet_kind: "metadata",
                got: packet_size,
            });
        } else if trace.buf.len() < packet_size {
            return Err(ParseError::MissingPacketData {
                want: packet_size,
                got: trace.buf.len(),
            });
        }

        let (left, option_name) = match parse_string(left) {
            Ok((left, option_name)) => (left, option_name),
            Err(err) => match err {
                StringParseError::TooSmall => {
                    return Err(ParseError::StringTooSmall {
                        packet_kind: "metadata",
                        field: "option name",
                    })
                }
                StringParseError::InvalidUTF8 => {
                    return Err(ParseError::InvalidString {
                        packet_kind: "metadata",
                        field: "option name",
                    })
                }
            },
        };

        match option_name {
            "epoch" if left.len() <= 8 => Err(ParseError::MissingPacketData {
                got: left.len(),
                want: 8,
            }),
            "epoch" => {
                let (_, nanos) = parse_u64(left);
                trace.epoch = SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos);
                // TODO: check that all bytes according to packet_size are
                // processed.
                trace.buf.drain(..packet_size);
                Ok(())
            }
            _ => Err(ParseError::UnknownOption(option_name.to_owned())),
        }
    }

    fn parse_event_packet(&mut self) -> Result<Event, ParseError> {
        let trace = &mut *self.trace;
        debug_assert_eq!(trace.buf[0..4], EVENT_MAGIC.to_be_bytes());

        let (left, packet_size) = parse_u32(&trace.buf[4..]);
        let packet_size = packet_size as usize;
        if packet_size <= MIN_EVENT_PACKET_SIZE {
            return Err(ParseError::PacketTooSmall {
                packet_kind: "event",
                got: packet_size,
            });
        } else if trace.buf.len() < packet_size {
            return Err(ParseError::MissingPacketData {
                got: trace.buf.len(),
                want: packet_size,
            });
        }

        let (left, stream_id) = parse_u32(&left[..packet_size - 8]);
        let (left, stream_counter) = parse_u32(left);
        let (left, substream_id) = parse_u64(left);
        let (left, start) = parse_timestamp(left, trace.epoch);
        let (left, end) = parse_timestamp(left, trace.epoch);
        let (left, description) = match parse_string(left) {
            Ok((left, description)) => (left, description.to_owned()),
            Err(err) => match err {
                StringParseError::TooSmall => {
                    return Err(ParseError::StringTooSmall {
                        packet_kind: "event",
                        field: "description",
                    })
                }
                StringParseError::InvalidUTF8 => {
                    return Err(ParseError::InvalidString {
                        packet_kind: "event",
                        field: "description",
                    })
                }
            },
        };

        let mut attributes = Vec::new();
        let mut left = left;
        while !left.is_empty() {
            let attribute_name = match parse_string(left) {
                Ok((l, attribute_name)) => {
                    left = l;
                    attribute_name.to_owned()
                }
                Err(err) => match err {
                    StringParseError::TooSmall => {
                        return Err(ParseError::StringTooSmall {
                            packet_kind: "event",
                            field: "attribute name",
                        })
                    }
                    StringParseError::InvalidUTF8 => {
                        return Err(ParseError::InvalidString {
                            packet_kind: "event",
                            field: "attribute name",
                        })
                    }
                },
            };

            let attribute_value = match parse_value(left) {
                Ok((l, attribute_value)) => {
                    left = l;
                    attribute_value
                }
                Err(err) => match err {
                    ValueParseError::StringParseError(StringParseError::TooSmall) => {
                        return Err(ParseError::StringTooSmall {
                            packet_kind: "event",
                            field: "attribute value",
                        })
                    }
                    ValueParseError::StringParseError(StringParseError::InvalidUTF8) => {
                        return Err(ParseError::InvalidString {
                            packet_kind: "event",
                            field: "attribute value",
                        })
                    }
                    ValueParseError::UnknownType(byte) => {
                        return Err(ParseError::UnknownValueType(byte))
                    }
                },
            };

            attributes.push((attribute_name, attribute_value));
        }

        // TODO: check all bytes from packet are read.
        trace.buf.drain(..packet_size);

        Ok(Event {
            stream_id,
            stream_counter,
            substream_id,
            start,
            end,
            description,
            attributes,
        })
    }
}

/// Parse a single `u32` from `bytes`.
///
/// # Panics
///
/// Panics if `bytes` is less than 4 bytes long.
fn parse_u32(bytes: &[u8]) -> (&[u8], u32) {
    let n = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
    (&bytes[4..], n)
}

/// Parse a single `u64` from `bytes`.
///
/// # Panics
///
/// Panics if `bytes` is less than 8 bytes long.
fn parse_u64(bytes: &[u8]) -> (&[u8], u64) {
    let n = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    (&bytes[8..], n)
}

/// See [`parse_u64`].
fn parse_i64(bytes: &[u8]) -> (&[u8], i64) {
    let n = i64::from_be_bytes(bytes[0..8].try_into().unwrap());
    (&bytes[8..], n)
}

/// See [`parse_u64`].
fn parse_f64(bytes: &[u8]) -> (&[u8], f64) {
    let n = f64::from_be_bytes(bytes[0..8].try_into().unwrap());
    (&bytes[8..], n)
}

/// Parse a single timestamp from `bytes`.
///
/// # Panics
///
/// Panics if `bytes` is less than 8 bytes long.
fn parse_timestamp(bytes: &[u8], epoch: SystemTime) -> (&[u8], SystemTime) {
    let (left, nanos) = parse_u64(bytes);
    let timestamp = epoch + Duration::from_nanos(nanos);
    (left, timestamp)
}

/// Parse a single string from `bytes`.
///
/// # Panics
///
/// Panics if `bytes` is less than 2 bytes long.
fn parse_string(bytes: &[u8]) -> Result<(&[u8], &str), StringParseError> {
    let len = u16::from_be_bytes(bytes[0..2].try_into().unwrap()) as usize;
    if bytes.len() - 2 < len {
        Err(StringParseError::TooSmall)
    } else {
        let (string, left) = bytes[2..].split_at(len);
        match str::from_utf8(string) {
            Ok(string) => Ok((left, string)),
            Err(..) => Err(StringParseError::InvalidUTF8),
        }
    }
}

enum StringParseError {
    TooSmall,
    InvalidUTF8,
}

/// Parse a single value from `bytes`.
fn parse_value(bytes: &[u8]) -> Result<(&[u8], Value), ValueParseError> {
    match bytes[0] {
        0b001 => {
            let (left, value) = parse_u64(&bytes[1..]);
            Ok((left, Value::Unsigned(value)))
        }
        0b010 => {
            let (left, value) = parse_i64(&bytes[1..]);
            Ok((left, Value::Signed(value)))
        }
        0b011 => {
            let (left, value) = parse_f64(&bytes[1..]);
            Ok((left, Value::Float(value)))
        }
        0b100 => match parse_string(&bytes[1..]) {
            Ok((left, value)) => Ok((left, Value::String(value.to_owned()))),
            Err(err) => Err(ValueParseError::StringParseError(err)),
        },
        byte => Err(ValueParseError::UnknownType(byte)),
        // TODO: parse slice of values.
    }
}

enum ValueParseError {
    StringParseError(StringParseError),
    UnknownType(u8),
}

#[derive(Debug)]
pub enum ParseError {
    IO(io::Error),
    MissingPacketData {
        got: usize,
        want: usize,
    },
    InvalidMagic(u32),
    PacketTooSmall {
        packet_kind: &'static str,
        got: usize,
    },
    StringTooSmall {
        packet_kind: &'static str,
        field: &'static str,
    },
    InvalidString {
        packet_kind: &'static str,
        field: &'static str,
    },
    UnknownOption(String),
    UnknownValueType(u8),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ParseError::*;
        match self {
            IO(err) => write!(f, "error reading trace: {err}"),
            MissingPacketData { got, want } => {
                write!(f, "missing packet data, want {want} bytes, got {got} bytes")
            }
            InvalidMagic(got_magic) => {
                write!(f, "packet has invalid magic value '{got_magic:#}'")
            }
            PacketTooSmall { packet_kind, got } => {
                write!(f, "{packet_kind} packet size too small, got {got} bytes")
            }
            StringTooSmall { packet_kind, field } => write!(
                f,
                "missing string data in {packet_kind} packet, {field} field",
            ),
            InvalidString { packet_kind, field } => {
                write!(f, "invalid string in {packet_kind} packet, {field} field")
            }
            UnknownOption(option_name) => write!(f, "unknown option name '{option_name}'"),
            UnknownValueType(byte) => write!(f, "unknown value type byte '{byte:#}'"),
        }
    }
}

#[derive(Debug)]
pub struct Event {
    stream_id: u32,
    #[allow(dead_code)] // Currently unused.
    stream_counter: u32,
    substream_id: u64,
    start: SystemTime,
    end: SystemTime,
    description: String,
    attributes: Vec<(String, Value)>,
}

#[derive(Debug)]
pub enum Value {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
}
