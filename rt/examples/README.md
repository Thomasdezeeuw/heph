# Examples

This directory contains a number of examples that highlight certain parts of the
system.


## 5. Remote Actor References

TODO: reimplement this.


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
