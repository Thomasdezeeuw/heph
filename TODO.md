# TODO

1. Fix example 2.
2. Write (much) more tests.

Stuff left to do.

 - TcpListener currently registers itself with a different id then the process
   is created with, causing it to never be scheduled.
 - Update to standard library's futures types.
 - Create a new type `SupervisorRef` that can be used in `ActorOptions` to use
   as supervisor, the supervisor itself must be added to the actor system.
   Default value will be none, using the default supervisor (LogSupervisor).
   Maybe remove NopSupervisor?
 - Change Process type to only have a `run` method. Then create a new struct
   ProcessData<P: Process> { priority: Priority, runtime: Duration, process: P },
   which the Scheduler uses in `Box<ProcessData<dyn Process>>`. To at least
   deduplicate the code.
 - Initiator configuration: call init before or after fork?
 - Use cpu core affinity after forking?
 - For sending messages don't use `mio::Poll` notifications, just schedule the
   process.

## Tests

Most types should be `!Send`, make sure they are.

## Metrics/system messages

Currently the `Actor` trait has a single `Message` definition, maybe this should
be expanded to allow for system message like metrics.

## net module

With at least TcpListener and TcpStream. Prefer to use the standard library
types, rather then a wrapper around it, see mio-st net module. Use the
`NewActor` trait to create an `Actor` per incoming connection. Let `TcpListener`
implement `Initiator`.

## More logging

In general the `ActorSystem` needs more logging, mostly debug level. Maybe
provide a default logging implementation (behind a feature flag) that does
logging via the `log` crate.

## Make Actor messages Clone

This way if an actor crashes, the message can be delivered again.

## Improved scheduler

One that does adhere to the priority. How do `Initators` fit in the scheduler,
if at all? Should a low priority in any case, to not flood the server with new
requests while not handling older ones.

## Improve futures `Waker` implementation

Currently it creates an event in `mio-st`, maybe it could just directly schedule
the process.

## Pass errors to supervisor

Currently the `Supervisor` is an unused trait, the `ActorProcess` should pass
the generated error to a supervisor.

## Multi processes

Create multiple process, see `f-test`.

## (Possible) optimisations

Consider using `repr(transparent)`.

## Timers

`mio-st` has timers, expose them somehow. Likely via a future.

## CI

Enable Travis.

## Mutation testing

Use https://github.com/llogiq/mutagen for mutation testing.

## Traces

Add a way to create traces on the running system to gain insight into it.


# Ideas

Some ideas for examples.

## URL checker

A (limited) number of actors that retrieves a page. 1 actor that filters the
URLs, so an URL is only visited once.

## Log proxy

Accepts log over TCP and/or UDP and stores them in SQL database, e.g. Postgres.
A single UDP packet using something like hmac for authentication.
