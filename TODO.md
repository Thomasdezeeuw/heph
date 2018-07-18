# TODO

Stuff left to do.

 - Remove `'static` lifetime from actors/initiators, starting in the scheduler.

 - Implement TcpStream.connect.
 - Add examples to TcpListener and TcpStream (connect).
 - implement Executor for ActorSystemRef.
 - Create a new type `SupervisorRef` that can be used in `ActorOptions` to use
   as supervisor, the supervisor itself must be added to the actor system.
   Default value will be none, using the default supervisor (LogSupervisor).
 - Initiator configuration: call init before or after fork?
 - Use cpu core affinity after forking?
 - Write (unit) tests for each module.

## Metrics/system messages

Currently the `Actor` trait has a single `Message` definition, maybe this should
be expanded to allow for system message like metrics.

## net module

Add UDP types.

Add TcpStream.connect.

Document TcpStream.

Add examples to:
 - TcpListener,
 - TcpStream.

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

# Tests

What needs (more) testing.

 - The actor module.
 - The scheduler module.
 - The system module.
 - The net module.
 - Most types should be `!Send`, make sure they are.
 - If an InitiatorProcess returns an error; now we have 0 initiators -> actor
   system should stop, returning `Ok(())`.
 - TaskProcess: usage of the provided `task::Context`.
