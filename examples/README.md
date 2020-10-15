# Examples

This directory contains a number of examples that highlight certain parts of the
system.

## 1. Hello World

Conforming to the tradition that is "Hello World", a simple program that prints
"Hello World".

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".


## 1b. Hello World, again

This example is the same as example 1, but uses the `schedule` actor option.
This shows the event driven nature of Heph. Actors by default are not scheduled
and run, but need an external event, such as sending them a message, to active
and run them.

The code can be found in `1b_hello_world.rs` and run with `cargo run --example
1b_hello_world`, and it should print "Hello World".


## 2. Ip server

The second example is a simple TCP server that writes the ip of the connection
to the connection.

The code can be found in `2_my_ip.rs` and run with `cargo run --example
2_my_ip`, running something like `nc localhost 7890` should then print your ip
address, e.g. "127.0.0.1".


## 3. RPC

Example three shows how Heph makes Remote Procedure Calls (RPC) easy.


## 4. Synchronous actor

The fourth example how to use synchronous actors. These are actors that have the
thread all to themselves, which means that can do heavy computation and blocking
I/O without stalling other actors.


## 5. Remote actor references

Examples 5a and 5b should be run together. Example 5a shows how to create a
`RemoteRelay` which allow remote actors to send messages to local actors.
Example 5b shows how to create and use an remote actor reference, referencing
the actor created in example 5a.

## 6. Process signal handling

Heph has build-in support for handling process signals. This example shows this
can be used to cleanly shutdown your application.

The code can be found in `6_process_signals.rs` and run with `cargo run
--example 6_process_signals`, pressing ctrl-c (sending it an interrupt signal
`SIGINT`) should shutdown the example cleanly.
