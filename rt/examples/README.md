# Examples

This directory contains a number of examples that highlight certain parts of the
system.


## 1. Hello World

Conforming to the tradition that is "Hello World", a simple program that prints
"Hello World".

The code can be found in `1_hello_world.rs` and run with `cargo run --example
1_hello_world`, and it should print "Hello World".


## 2. IP Server

The second example is a simple TCP server that writes the ip of the connection
to the connection.

The code can be found in `2_my_ip.rs` and run with `cargo run --example
2_my_ip`, running something like `nc localhost 7890` should then print your ip
address, e.g. "127.0.0.1".


## 3. RPC

Example three shows how Heph makes Remote Procedure Calls (RPC) easy.


## 4. Synchronous Actor

The fourth example how to use synchronous actors. These are actors that have the
thread all to themselves, which means that can do heavy computation and blocking
I/O without stalling other actors.


## 5. Remote Actor References

TODO: reimplement this.


## 6. Process Signal Handling

Heph has build-in support for handling process signals. This example shows this
can be used to cleanly shutdown your application.

The code can be found in `6_process_signals.rs` and run with `cargo run
--example 6_process_signals`, pressing ctrl-c (sending it an interrupt signal
`SIGINT`) should shutdown the example cleanly.


## 7. Restart Supervisor Macro

Example seven shows how the `restart_supervisor!` macro can be used to easily
create a new `Supervisor` implementation that attempts to restart the actor with
cloned arguments.


## 8. Runtime Tracing

Heph supports generating trace files in its own custom format, described in the
[Trace Format design document]. This format can be converted into [Chrome's
Trace Event Format] so it can be opened by [Catapult trace view].

```bash
 $ cargo run --example 8_tracing       # Run the example, to generate the trace.
 $ cd ../tools                         # Got into the tools directory.
                                       # Convert the trace to Chrome's format.
 $ cargo run --bin convert_trace ../rt/heph_tracing_example.bin.log
                                       # Make the trace viewable in HTML.
 $ $(CATAPULT_REPO)/tracing/bin/trace2html ../heph_tracing_example.json
 $ open ../heph_tracing_example.html   # Finally open the trace in your browser.
```

[Trace Format design document]: ../doc/Trace%20Format.md
[Chrome's Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview
[Catapult trace view]: https://chromium.googlesource.com/catapult/+/refs/heads/master/tracing/README.md

## 9. Systemd support

Heph also has various utilties to support [systemd]. You can use the following
[service] file to start example 9.

[systemd]: https://systemd.io
[service]: https://www.freedesktop.org/software/systemd/man/systemd.service.html

```
[Unit]
Description=Heph Test service
Requires=network.target

[Service]
# Required to setup communication between the service manager (systemd) and the
# service itself.
Type=notify
# Restart the service if it fails.
Restart=on-failure
# Require the service to send a keep-alive ping every minute.
WatchdogSec=1min

# Path to the example.
ExecStart=/path/to/heph/target/debug/examples/9_systemd
# The port the service will listen on.
Environment=PORT=8000
# Enable some debug logging.
Environment=DEBUG=1
```
