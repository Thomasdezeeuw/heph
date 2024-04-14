# Examples

This directory contains a number of examples that highlight certain parts of
Heph. Note that the Heph-rt crate has even more examples in its [examples
directory].

[examples directory]: ../rt/examples


## 1. Hello World

Conforming to the tradition that is "Hello World", a simple program that prints
"Hello World".

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".


## 2. RPC

The second example shows something more interesting: how Heph makes Remote
Procedure Calls (RPC) easy.


## 3. Synchronous Actor

The third example shows how to use synchronous actors. These are actors that
have the thread all to themselves, which means that can do heavy computation and
blocking I/O without stalling other actors.


## 4. Restart Supervisor Macro

Example four shows how the `restart_supervisor!` macro can be used to easily
create a new `Supervisor` implementation that attempts to restart the actor with
cloned arguments.
