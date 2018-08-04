//! Tests for the process module.

use std::io;
use std::future::Future;
use std::mem::PinMut;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use crossbeam_channel as channel;
use mio_st::event::EventedId;
use mio_st::poll::Poller;

use crate::initiator::Initiator;
use crate::process::{ProcessId, Process, ProcessResult, InitiatorProcess, TaskProcess};
use crate::system::ActorSystemRef;
use crate::util::Shared;
use crate::waker::new_waker;

#[test]
fn pid_to_evented_id() {
    let pid = ProcessId(0);
    let id: EventedId = pid.into();
    assert_eq!(id, EventedId(0));
}

#[test]
fn evented_id_to_pid() {
    let id = EventedId(0);
    let pid: ProcessId = id.into();
    assert_eq!(pid, ProcessId(0));
}

#[test]
fn pid_display() {
    assert_eq!(ProcessId(0).to_string(), "0");
    assert_eq!(ProcessId(100).to_string(), "100");
    assert_eq!(ProcessId(8000).to_string(), "8000");
}

struct SimpleInitiator {
    called: Shared<usize>,
}

impl Initiator for SimpleInitiator {
    fn init(&mut self, _: &mut Poller, _: ProcessId) -> io::Result<()> {
        unreachable!();
    }

    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        *self.called.borrow_mut() += 1;
        match *self.called.borrow() {
            1 => Ok(()),
            2 => Err(io::ErrorKind::Other.into()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn initiator_process() {
    let mut system_ref = ActorSystemRef::test_ref();

    let called = Shared::new(0);
    let mut process = InitiatorProcess::new(SimpleInitiator { called: called.clone() });

    // Ok run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(*called.borrow(), 1, "expected the process to be run");

    // Error run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
    assert_eq!(*called.borrow(), 2, "expected the process to be run");
}

struct ErroneousInitiator;

impl Initiator for ErroneousInitiator {
    fn init(&mut self, _: &mut Poller, _: ProcessId) -> io::Result<()> {
        unreachable!();
    }

    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        Err(io::ErrorKind::Other.into())
    }
}

#[test]
fn erroneous_initiator_process() {
    let mut system_ref = ActorSystemRef::test_ref();

    let mut process = InitiatorProcess::new(ErroneousInitiator);

    // If Initiator returns an error it should return Complete.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
}

struct TaskFuture {
    called: Arc<AtomicUsize>,
}

impl Future for TaskFuture {
    type Output = ();
    fn poll(self: PinMut<Self>, _ctx: &mut Context) -> Poll<Self::Output> {
        match self.called.fetch_add(1, Ordering::SeqCst) {
            0 => Poll::Pending,
            1 => Poll::Ready(()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn task_process() {
    let mut system_ref = ActorSystemRef::test_ref();

    let called = Arc::new(AtomicUsize::new(0));
    let task = Box::new(TaskFuture { called: Arc::clone(&called) }).into();
    let (send, _recv) = channel::unbounded();
    let waker = new_waker(ProcessId(0), send);
    let mut process = TaskProcess::new(task, waker);

    // Pending run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(called.load(Ordering::SeqCst), 1, "expected the process to be run");

    // Ready run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
    assert_eq!(called.load(Ordering::SeqCst), 2, "expected the process to be run a second time");
}
