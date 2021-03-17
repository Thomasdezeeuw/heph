# Trace Format

###### Version 0.1.0.

This design document describes the format used by Heph's trace system.

It's inspired by the Common Trace Format (v1.8.3), available here:
<https://diamon.org/ctf>.

## Overview

A single trace is split into multiple packets, which allows them to be easily
send using packet based protocols, such as UDP, and stream based protocols, such
as TCP, or for writing to disk. There are two kinds of packets:

 * Metadata packets, used for setting options.
 * Event packets, which contains information about a single event.

A trace is conceptually split into multiple streams. A stream represents a
sequence of events (not metadata), from the same thread of execution. For
example events that happen on the same thread. Each stream has a 32 bit
identifier (id) unique in the trace, which is used to coalesce events.

Streams can optionally be split into multiple substreams which represent
multiplexed execution on the same stream of execution, e.g. as used by
co-routines, green threads or many asynchronous programming implementations that
schedule multiple tasks on the same thread.

The format always uses big-endian for integers.

## Metadata Packet

Metadata packets can set a certain option for the entire trace, i.e. it involves
all other packets. They may be present in the trace at any point, but are
usually at the start of it. Metadata packets have to following layout:

```
Bits		Description.
[ 0..31]	Magic constant (0x75D11D4D) to indicate a metadata packet.
[31..63]	Size of the packet in bytes.
[64..79]	Option name length, unsigned 16 bit integer.
[80..$n]	Option name in UTF-8, length defined by the option length.
[$n+1..]	Option value, layout depends on the option.
```

The packet starts with a 32 bit magic constant `0x75D11D4D`[^1], allowing the
packet to be differentiated from event packets (see below). The next 32 bits are
the size of the packet in bytes (including itself and the magic constant) as a
unsigned 32 bit integer (in big-endian, same as all integers).

A single metadata packet can only specify a single option. This is done by
giving the option name as a string: 16 unsigned bits to indicate the length of
the string, followed by that many bytes in UTF-8. Finally the value for the
option is supplied, the layout of which depends on the option itself.

The layout would something like the following as a structure in Rust:

```
struct Metadata<Option> {
	magic: u32,
	size:  u32,
	option_name_length: u16,
	option_name:       [u8],
	option_value:    Option, // Layout depends on the option.
}
```

### Options

Currently the following options are supported:

 * `epoch`: Sets the epoch (zero time) used in event packets. The value must
   specified as the number of nanoseconds since the Unix epoch as unsigned 64
   bit integer. This will be used as wall-clock based epoch, while the timings
   in the events can be based on monotonic clocks (which often don't have a
   relation to the wall-clocks).

Note that no options are required to be set on the trace, meaning a trace
without any metadata packets is valid.

### Example

The following example shows the layout for a metadata packet setting the `epoch`
option.

```
Bits		Value		Description.
[ 0..31]	0x75D11D4D	Magic constant to indicate a metadata packet.
[31..63]	23  		Size of the packet: 4 (magic) + 4 (packet size) +
                		2 (option name length) + 5 (option name) + 8 (option value) = 23 bytes.
[64..79]	5   		Option length: "epoch" is 5 bytes.
[80..$n]	"epoch"		Option name.
[$n+1..]	1610113734118010000	Option value: 08 Jan 2021.
```

## Event Packet

An event packet describes a single event. The layout is the following:

```
Bits		Description.
[  0.. 31]	Magic constant (0xC1FC1FB7) to indicate an event packet.
[ 32.. 63]	Size of the packet in bytes.
[ 64.. 95]	Stream identifier (id).
[ 96..127]	Stream event counter.
[128..191]	Substream identifier (id).
[192..255]	Start time in nanoseconds, relative to trace's epoch.
[256..319]	End time in nanoseconds, relative to trace's epoch.
[320..335]	Desciption length, unsigned 16 bit integer.
[336.. $n]	Description in UTF-8, length defined by the description length.
[$n+1..$k]	Attributes, optional, see below.
```

An event packet also starts with a 32 bit magic constant `0xC1FC1FB7` [^2] and
the size of the packet in bytes as 32 bit unsigned integer.

Next are 32 bits for the stream id, interrupted as an 32 bit unsigned integer.
This stream id can be used to coalesce events. For example if we have two events
with the same stream id, where event A start at time 10 and ends at time 20 and
an event B start starts at the time 5 (i.e. earlier than event A) and ends at
time 15 (i.e. later than event B) we have an implicit parent-child relation
between the two events, event B caused event A. This allows us to create a
call-stack like image of the events, with the code needed to know anything about
it's caller/parent (i.e. we don't have to carry a `Span` like structure around
in the code).

The fourth field is the stream event counter. This simply a 32 bit unsigned
integer which count the events in the stream. This can be used in packet based
protocols to detect if an event was missed. The counter may wrap-around on
overflow, i.e. after `2 ^ 32` comes `0`. Note however this is optional and may
be ignore by consumers of the trace.

The fifth field is the substream id, a 64 bit unsigned integer. Similar to the
stream id, this can be used to coalesce events, of multiplexed threads of
execution on top of the stream.

The next two fields are the start and end timestamps of the event. Both are 64
bit unsigned integers representing the number of nanoseconds since the epoch of
the trace. Also see the `epoch` option of the metadata packet.

Next comes a description in string format: 16 unsigned bits to indicate the
length and that many bytes in UTF-8.

The above fields are all required, the following attributes however are not. An
event with zero attributes is valid.

The attributes are key-value pairs with additional information about the event.
The following describes the layout of the attributes. **Note** that the bits
below start at zero, but are actually after the other event field (see above).

```
Bits			Description.
[   0..  15]	Attribute name length, unsigned 16 bit integer.
[  16..  $n]	Attribute name in UTF-8, length defined by the attribute name length.
[$n+1..$n+8]	Attribute value type, a byte to indicate the type of the attribute value.
[$n+9..$k  ]	Attribute value, layout depends on the type of the value.
```

Each attribute is a key-value pair, so it start with the name of the attribute
as a string. A string, like the event description, is 16 unsigned bits followed
by that many bytes in UTF-8 that make up the attribute name. Next is the
attribute value.

The attribute value can one of the following types.

```
Byte layout	Type
0b0000_0001	Unsigned 64 bit integer.
0b0000_0010	Signed 64 bit integer.
0b0000_0011	64 bit floating point.
0b0000_0100	String: 16 bit unsigned integer as length, followed by that many bytes.

0b1000_0000	Array marker: 16 bit unsigned integer to indicate the length, followed
           	by that many value laid out a definied by the value type above.
```

The unsigned integer, signed integer and floating point values are always 64
bits and in big-endian format.

The string type we've already seen previous: 16 bit unsigned integer to indicate
the length, followed by that many bytes in UTF-8 which is the string content.

Finally dynamically sized arrays are supported. Within the array only a single
type is support, i.e. it's not supported to use different types within the same
array. The format of the array is an 16 unsigned bit integer to indicate the
length followed by that many values. The layout of the values is determined by
the type of the value.

It's not valid to have an untyped array, an array must always be combined with a
value type. For example `0b0000_0100 | 0b1000_0000` indicates an array of
strings. **The array type on it's own, i.e.** `0b1000_0000`**, is invalid**.

The layout would something like the following as a structure in Rust:

```
struct Event<Attributes> {
	magic: u32,
	size:  u32,
	description_length: u16,
	description:       [u8],
	attributes: Attributes, // Layout differs per event.
}

// NOTE: the integers and floats have the same layout as `u64`/`i64`/`f64` from
// big-endian bytes.

struct StringAttribute {
	length: u16,
	value: [u8],
}

struct ArrayAttribute<A> {
	length: u16,
	values: [A],
}
```

### Example

The following example shows the layout for an event packet.

```
Bits		Value		Description.
[  0.. 31]	0xC1FC1FB7	Magic constant to indicate an event packet.
[ 31.. 63]	91  		Size of the packet (728 / 8).
[ 64.. 95]	0   		Stream identifier (id).
[ 96..127]	0   		Stream event counter.
[128..191]	1   		Substream identifier (id).
[192..255]	100 		Start time.
[256..319]	200 		End time.
[320..335]	8   		Desciption length.
[336..399]	"My event"	Description.
[400..415]	4   		Attribute name length.
[416..447]	"Test"		Attribute name.
[448..455]	0b0000_0001	Attribute value type: unsigned 64 bit integer.
[456..519]	123 		Attribute value.
[520..535]	5   		Attribute name length.
[536..575]	"Test2"		Attribute name.
[576..583]	0b1000_0011	Attribute value type: array of 64 floats.

[584..599]	2   		Array length.
[600..663]	123.456		Array[0].
[664..727]	789.0		Array[1].
```


[^1]: The value is `0x75D11D57` minus 10.

[^2]: The value is `0xC1FC1FC1` minus 10.
