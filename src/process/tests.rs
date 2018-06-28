//! Tests for the process module.

use std::io;
use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;

use mio_st::event::EventedId;
use mio_st::poll::Poller;

use actor::{Actor, ActorContext, ActorResult, Status};
use initiator::Initiator;
use process::{ProcessId, EmptyProcess, Process, ProcessResult, ActorProcess, InitiatorProcess};
use system::{ActorSystemBuilder, ActorSystemRef, ActorOptions};

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

#[test]
#[should_panic(expected = "can't run empty process")]
fn cant_run_empty_process() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();
    let _ = EmptyProcess.run(&mut system_ref);
}

struct SimpleInitiator {
    called: Rc<RefCell<bool>>,
}

impl Initiator for SimpleInitiator {
    fn init(&mut self, _: &mut Poller, _: ProcessId) -> io::Result<()> {
        unreachable!();
    }

    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        *self.called.borrow_mut() = true;
        Ok(())
    }
}

#[test]
fn initiator_process() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let called = Rc::new(RefCell::new(false));
    let mut process = InitiatorProcess::new(SimpleInitiator { called: Rc::clone(&called) });

    // If Initiator returns an error it should return Pending.
    if let ProcessResult::Complete = process.run(&mut system_ref) {
        panic!("expected ProcessResult::Pending");
    }

    assert_eq!(*called.borrow(), true, "expected the process to be run");
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
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let mut process = InitiatorProcess::new(ErroneousInitiator);

    // If Initiator returns an error it should return Complete.
    if let ProcessResult::Pending = process.run(&mut system_ref) {
        panic!("expected ProcessResult::Complete");
    }
}
