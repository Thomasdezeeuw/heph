# Scheduler Design

The design of the scheduler is based on Linux' Completely Fair Scheduler (CFS).
[1]

Processes are ordered based on fair\_runtime, that is `runtime * priority`. For
example If a process runs for 10 milliseconds with a normal priority (10) it's
fair\_runtime will be 100 milliseconds. The process with the lowest
fair\_runtime that is ready will be run next.

Since processes (well futures) aren't preemptible they can run for ever if they
want, it's up to the developer to make sure they don't. However if a long
running process does at some point return then at least it won't be scheduled
again quickly after, thus reducing the impact of badly behaving processes.

Using this way of scheduling processes that do very little CPU intensive work
and mostly do I/O (and thus go into a unready state very quickly) will be
favoured in this algorithm.

Scheduling processes to be run is based on `ProcessId`. There are currently
three sources of processes being scheduled;

 - The system poller (`mio`). The system poller is used to receive events about
   system resources, e.g. TCP sockets, and can trigger the scheduling of a
   process. Here a batch of processes are scheduled at the same time, without a
   process running.

 - Sending message to an actor (process). This is only possible for
   `ActorPrcocess`es. This can be done when another process is running.

 - Future's `Waker`. Actors can wake itself or another actor via the future's
   `Waker`. This can be done when another process is running.

The scheduler must also be able to add new process, without it it wouldn't be
very useful. But it must be able to this while a process is running, e.g. in
case of an `Initiator` it must possible for it to add an actor to the system.
Effectively we need mutable access to a process in the scheduler to run it,
while being able to add another process to it.

Which brings us back to the scheduling of processes. The last two scheduling
sources, sending messages and the futures' `Waker`, can be implemented via two
methods, 1) directly on the scheduler, or 2) via the system poller.

Scheduling directly on the scheduler would avoid some overhead and would likely
be faster. However letting the system poller deal with it is easier to implement
(it is already implemented in fact), and it reduce the need for mutable access
to the scheduler while a process is running.


## Requirements

The following operations need to be supported, and need to be optimised for.

 - Scheduling of a process, based on `ProcessId`, possibly while a process
   (possibly itself) is running.
 - Adding new processes while another process is running.
 - Getting the next process to run, based on the lowest fair\_runtime.


## Priority

The priority of a process influence how often a process is run and the time
between two runs. Party due to this it is a good idea to give `Initiators` a low
priority to make sure that already accepted requests are handled before new
requests are accepted, to prevent the system being flooded with new requests
without the older requests being processed.


## Open questions

 - When to run a process vs. calling the system poller. Should this be an
   option? Maybe poll userspace events more often, would need to be added to
   mio?
 - What data structure to use for the `Scheduler`, Linux' CFS uses a Red-black
   tree.
 - How to schedule a process, when sending message or using a futures' `Waker`,
   directly on the scheduler or via the system poller.
 - Can (very) long process overflow the `Duration` type, especially with the
   priority multiplier? Thinking along the lines of processes that should be
   able to run for year on end.


## Resources

Completely Fair Scheduler on Wikipedia. [1]

Inside the Linux 2.6 Completely Fair Scheduler, by M. Tim Jones. [2]

[1]: https://wikipedia.org/wiki/Completely_Fair_Scheduler
[2]: https://www.ibm.com/developerworks/linux/library/l-completely-fair-scheduler
