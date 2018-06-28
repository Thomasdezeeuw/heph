# Examples

This directory contains a number of examples that highlight certain parts of the
system.

## 1. Hello World

Conforming with tradition the first example is "Hello World", a simple program
that prints "Hello World". Coming in at 62 lines is quite a large example for
such a simple program. This is because the system isn't designed for "print to
screen" problem domain. But it does illustrate the basic functionality of the
system.

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".

In the `b` variant of this example we use the `ActorFn` helper to reduce the
example to 30 lines. An improvement over the original, but that is still 10
times more then the required 3. The code for this can be found in
`1b_hello_world.rs`.

## 2. Ip server

The second example is a simple TCP server that writes the ip of the connection
to the connection.

The code can be found in `2_my_ip.rs` and run with `cargo run --example
2_my_ip`, running something like `nc -v localhost 7890` should then print your
ip address, e.g. "127.0.0.1".

## 3. Echo server

The next example will be a bit more complex; an echo server written using TCP.

The code can be found in `3_echo_server.rs` and run with `cargo run --example
3_echo_server`, running something like `echo "Hello world" | nc -v localhost
7890` should then echo back (print) "Hello world".
