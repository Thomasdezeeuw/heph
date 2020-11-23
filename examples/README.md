# Examples

This directory contains a number of examples that highlight certain parts of the
system.


## 1. Hello World

Conforming to the tradition that is "Hello World", a simple program that prints
"Hello World".

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".


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

TODO: reimplement this.


## 6. Process signal handling

Heph has build-in support for handling process signals. This example shows this
can be used to cleanly shutdown your application.

The code can be found in `6_process_signals.rs` and run with `cargo run
--example 6_process_signals`, pressing ctrl-c (sending it an interrupt signal
`SIGINT`) should shutdown the example cleanly.
