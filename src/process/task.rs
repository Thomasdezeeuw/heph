//! Module containing the implementation of the `Process` trait for
//! `FutureObj`s.

use std::future::{Future, FutureObj};
use std::mem::PinMut;
use std::task::{Context, LocalWaker, Poll};

use crate::process::{Process, ProcessResult};
use crate::system::ActorSystemRef;

/// A process that represent a `FutureObj` (used to be `TaskObj`).
#[derive(Debug)]
pub struct TaskProcess {
    /// The underlying task.
    task: FutureObj<'static, ()>,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl TaskProcess {
    /// Create a new `TaskProcess`.
    pub const fn new(task: FutureObj<'static, ()>, waker: LocalWaker) -> TaskProcess {
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
