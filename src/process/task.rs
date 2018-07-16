//! Module containing the implementation of the `Process` trait for
//! `TaskObj`s.

use std::future::{Future, FutureObj};
use std::mem::PinMut;
use std::task::{Context, LocalWaker, Poll};

use mio_st::registration::Registration;

use process::{Process, ProcessResult};
use system::ActorSystemRef;

/// A process that represent a `TaskObj`.
///
/// It calls `poll` until it returns `Poll::Ready` after which `run` returns
/// `ProcessResult::Complete`.
#[derive(Debug)]
pub struct TaskProcess {
    /// The underlying task.
    task: FutureObj<'static, ()>,
    /// Needs to stay alive for the duration of the task.
    registration: Registration,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl TaskProcess {
    /// Create a new `TaskProcess`.
    pub const fn new(task: FutureObj<'static, ()>, registration: Registration, waker: LocalWaker) -> TaskProcess {
        TaskProcess {
            task,
            registration,
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
