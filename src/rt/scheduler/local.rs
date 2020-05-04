//! Module with the thread-local scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn_local`].

use std::borrow::Borrow;
use std::collections::{BinaryHeap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::time::Duration;

use fnv::FnvBuildHasher;
use log::trace;

use crate::inbox::{Inbox, InboxRef};
use crate::rt::process::{ActorProcess, ProcessId};
use crate::rt::scheduler::{AddActor, Priority, ProcessData};
use crate::{NewActor, Supervisor};

pub(crate) struct LocalScheduler {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<ProcessData>>>,
    /// Inactive processes that are not ready to run.
    inactive: HashSet<Process, FnvBuildHasher>,
}

impl LocalScheduler {
    /// Create a new `LocalScheduler`.
    pub(super) fn new() -> LocalScheduler {
        LocalScheduler {
            ready: BinaryHeap::new(),
            inactive: HashSet::default(),
        }
    }

    /// Returns `true` if the schedule has any processes (in any state), `false`
    /// otherwise.
    pub(super) fn has_process(&self) -> bool {
        !self.inactive.is_empty() || self.has_ready_process()
    }

    /// Returns `true` if the schedule has any processes that are ready to run,
    /// `false` otherwise.
    pub(super) fn has_ready_process(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Add an actor to the scheduler.
    pub(super) fn add_actor<'s>(&'s mut self) -> AddActor<'s, HashSet<Process, FnvBuildHasher>> {
        AddActor {
            processes: &mut self.inactive,
            alloc: Box::new_uninit(),
        }
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(super) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = self.inactive.take(&pid) {
            self.ready.push(process.0)
        }
    }

    /// Returns the next ready process.
    pub(super) fn next_process(&mut self) -> Option<Pin<Box<ProcessData>>> {
        self.ready.pop()
    }

    /// Add back a process that was previously removed via
    /// [`LocalScheduler::next_process`].
    pub(super) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        let res = self.inactive.insert(Process(process));
        debug_assert!(res, "process with same pid already exists");
    }
}

impl fmt::Debug for LocalScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalScheduler")
    }
}

/// Wrapper around `Pin<Box<ProcessData>>` so we can implement traits on it.
// pub(crate) because its used in AddActor.
#[repr(transparent)]
pub(crate) struct Process(Pin<Box<ProcessData>>);

impl Eq for Process {}

impl PartialEq for Process {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ref().id() == other.0.as_ref().id()
    }
}

impl PartialEq<ProcessId> for Process {
    fn eq(&self, other: &ProcessId) -> bool {
        self.0.as_ref().id() == *other
    }
}

impl Hash for Process {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.0.as_ref().id().hash(state)
    }
}

impl Borrow<ProcessId> for Process {
    fn borrow(&self) -> &ProcessId {
        // See `ProcessData::id` why this is safe.
        unsafe { std::mem::transmute::<&Process, &ProcessId>(self) }
    }
}

impl<'s> AddActor<'s, HashSet<Process, FnvBuildHasher>> {
    /// Add a new inactive actor to the scheduler.
    pub(super) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Inbox<NA::Message>,
        inbox_ref: InboxRef<NA::Message>,
    ) where
        S: Supervisor<NA> + 'static,
        NA: NewActor + 'static,
    {
        let process = ProcessData {
            priority,
            fair_runtime: Duration::from_nanos(0),
            process: Box::pin(ActorProcess::new(
                supervisor, new_actor, actor, inbox, inbox_ref,
            )),
        };
        let AddActor {
            processes,
            mut alloc,
        } = self;
        let process: Pin<_> = unsafe {
            let _ = alloc.write(process);
            // Safe because we write into the allocation above.
            alloc.assume_init().into()
        };
        let res = processes.insert(Process(process));
        debug_assert!(res, "process with same pid already exists");
    }
}
