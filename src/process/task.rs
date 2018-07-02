//! Module containing the implementation of the `Process` trait for
//! `TaskObj`s.

use std::future::Future;
use std::mem::PinMut;
use std::task::{Context, LocalWaker, TaskObj, Poll};

use process::{Process, ProcessResult};
use system::ActorSystemRef;

/// A process that represent a `TaskObj`.
#[derive(Debug)]
pub struct TaskProcess {
    /// The underlying task.
    task: TaskObj,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl TaskProcess {
    /// Create a new `TaskProcess`.
    pub const fn new(task: TaskObj, waker: LocalWaker) -> TaskProcess {
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
