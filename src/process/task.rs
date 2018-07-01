//! Module containing the implementation of the `Process` trait for
//! `TaskObj`s.

use std::fmt;
use std::future::Future;
use std::mem::PinMut;
use std::task::{Context, LocalWaker, TaskObj, Poll};

use process::{Process, ProcessId, ProcessResult};
use system::{ActorSystemRef, Waker};

/// A process that represent a `TaskObj`.
pub struct TaskProcess {
    /// The underlying task.
    task: TaskObj,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl TaskProcess {
    /// Create a new `TaskProcess`.
    pub fn new(pid: ProcessId, task: TaskObj, system_ref: ActorSystemRef) -> TaskProcess {
        let waker = Waker::new(pid, system_ref);
        TaskProcess {
            task,
            waker,
        }
    }
}

impl Process for TaskProcess {
    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessResult {
        let mut ctx = Context::new(&self.waker, system_ref);
        match PinMut::new(&mut self.task).poll(&mut ctx) {
            Poll::Ready(()) => ProcessResult::Complete,
            Poll::Pending => ProcessResult::Pending,
        }
    }
}

impl fmt::Debug for TaskProcess {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TaskProcess")
            .finish()
    }
}
