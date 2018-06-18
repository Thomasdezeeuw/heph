# Examples

This directory contains a number of examples that highlight certain parts of the
system.

## Hello World

Conforming with tradition the first example is "Hello World", a simple program
that prints "Hello World". Coming in at 61 lines is quite a large example for
such a simple program. This is because the system isn't designed for "print to
screen" problem domain. But it does illustrate the basic functionality of the
system.

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".

## Echo server

The next actor will be a bit more complex; an echo server written using TCP.

The code can be found in `2_tcp_server.rs` and run with `cargo run --example
2_tcp_server`, running something like `echo "Hello world" | nc -v localhost
8080` should then echo back (print) "Hello world".
