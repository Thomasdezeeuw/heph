# Examples

This directory contains a number of examples that highlight certain parts of the
system.


## 5. Remote Actor References

TODO: reimplement this.


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
