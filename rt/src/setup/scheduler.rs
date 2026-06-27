//! Scheduler implementation.
//!
//! The scheduler is responsible for managing all processes in the runtime.
//! Keeping track of which processes are ready to run and which are waiting for
//! external events (such as incoming connections) to happen before continuing.
//! Furthermore it needs to determine which process to run next, i.e. ordering
//! of the processes.
//!
//! This is implemented using the following traits:
//!  * [`Scheduler`] is the data structure and algorithm that hold all
//!    processes.
//!  * [`Schedule`] can optionally used by a `Scheduler` to determine how to
//!    order processes.
//!  * [`Process`] represents a single process which can be added to the
//!    `Scheduler`.
//!  * [`RunnableProcess`] represents a process within the context of a
//!    `Scheduler` that can be run.
//!
//! See the documentation on the trait themselves for more information.
