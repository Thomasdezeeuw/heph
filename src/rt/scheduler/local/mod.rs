//! Module with the thread-local scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn_local`].
//!
//! [`RuntimeRef::try_spawn_local`]: crate::rt::RuntimeRef::try_spawn_local

use std::collections::BinaryHeap;
use std::fmt;
use std::pin::Pin;

use inbox::Manager;
use log::trace;

use crate::actor::context;
use crate::rt::process::{self, ActorProcess, ProcessId};
use crate::rt::scheduler::{self, AddActor, Priority};
use crate::{NewActor, Supervisor};

mod inactive;

use inactive::Inactive;

pub(super) type ProcessData = scheduler::ProcessData<dyn process::Process>;

pub(in crate::rt) struct LocalScheduler {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<ProcessData>>>,
    /// Inactive processes that are not ready to run.
    inactive: Inactive,
}

impl LocalScheduler {
    /// Create a new `LocalScheduler`.
    pub(in crate::rt) fn new() -> LocalScheduler {
        LocalScheduler {
            ready: BinaryHeap::new(),
            inactive: Inactive::empty(),
        }
    }

    /// Returns `true` if the schedule has any processes (in any state), `false`
    /// otherwise.
    pub(in crate::rt) fn has_process(&self) -> bool {
        self.inactive.has_process() || self.has_ready_process()
    }

    /// Returns `true` if the schedule has any processes that are ready to run,
    /// `false` otherwise.
    pub(in crate::rt) fn has_ready_process(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Add an actor to the scheduler.
    pub(in crate::rt) fn add_actor<'s>(
        &'s mut self,
    ) -> AddActor<&'s mut LocalScheduler, dyn process::Process> {
        AddActor {
            processes: self,
            alloc: Box::new_uninit(),
        }
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(in crate::rt) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = self.inactive.remove(pid) {
            self.ready.push(process)
        }
    }

    /// Returns the next ready process.
    pub(in crate::rt) fn next_process(&mut self) -> Option<Pin<Box<ProcessData>>> {
        self.ready.pop()
    }

    /// Add back a process that was previously removed via
    /// [`LocalScheduler::next_process`].
    pub(in crate::rt) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        self.inactive.add(process);
    }
}

impl fmt::Debug for LocalScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalScheduler")
    }
}

impl<'s> AddActor<&'s mut LocalScheduler, dyn process::Process> {
    /// Add a new inactive actor to the scheduler.
    pub(in crate::rt) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Manager<NA::Message>,
        is_ready: bool,
    ) where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Context = context::ThreadLocal> + 'static,
    {
        #[allow(trivial_casts)]
        debug_assert!(
            inactive::ok_ptr(self.alloc.as_ptr() as *const ()),
            "SKIP_BITS invalid"
        );

        let process = ProcessData::new(
            priority,
            Box::pin(ActorProcess::new(supervisor, new_actor, actor, inbox)),
        );
        let AddActor {
            processes,
            mut alloc,
        } = self;
        let process: Pin<_> = unsafe {
            let _ = alloc.write(process);
            // Safe because we write into the allocation above.
            alloc.assume_init().into()
        };
        if is_ready {
            processes.ready.push(process)
        } else {
            processes.inactive.add(process);
        }
    }
}
