Benchmarks for the container used in the timers implementation.

It focusses on three operations:
 * Removing the next timer to expire.
 * Adding a new timer, with an expectation that the deadline is after the last
   deadline added.
 * Removing an arbitrary timer.
