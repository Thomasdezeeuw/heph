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

## 3. Echo server

The next example will be a bit more complex; an echo server written using TCP.
However using all the utilities that `futures-util` has to offer this is
actually not that hard either.

One special feature this example uses is the "Actor Registry". The actor
registry allows an actor to be registered and for another actor to look up the
first actor and get a actor reference.

The code can be found in `3_echo_server.rs` and run with `cargo run --example
3_echo_server`, running something like `echo "Hello world" | nc -v localhost
7890` should then echo back (print) "Hello world".
