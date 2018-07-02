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
use system::error::{SendError, SendErrorReason};

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
    called: Rc<RefCell<usize>>,
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
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let called = Rc::new(RefCell::new(0));
    let mut process = InitiatorProcess::new(SimpleInitiator { called: Rc::clone(&called) });

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
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let mut process = InitiatorProcess::new(ErroneousInitiator);

    // If Initiator returns an error it should return Complete.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
}

struct SimpleActor {
    handle_called: Rc<RefCell<usize>>,
    poll_called: Rc<RefCell<usize>>,
}

impl SimpleActor {
    /// Returns (actor, handle called, poll called).
    fn new() -> (SimpleActor, Rc<RefCell<usize>>, Rc<RefCell<usize>>) {
        let handle_called = Rc::new(RefCell::new(0));
        let poll_called = Rc::new(RefCell::new(0));
        let actor = SimpleActor {
            handle_called: Rc::clone(&handle_called),
            poll_called: Rc::clone(&poll_called)
        };
        (actor, handle_called, poll_called)
    }
}

impl Actor for SimpleActor {
    type Message = ();
    type Error = !;

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        *self.handle_called.borrow_mut() += 1;
        Poll::Ready(Ok(Status::Ready))
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        *self.poll_called.borrow_mut() += 1;
        Poll::Ready(Ok(Status::Ready))
    }
}

#[test]
fn actor_process() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let (actor, handle_called, poll_called) = SimpleActor::new();
    let mut process = ActorProcess::new(ProcessId(0), actor,
        ActorOptions::default(), system_ref.clone());
    assert_eq!(*poll_called.borrow(), 0);

    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(*poll_called.borrow(), 1, "expected actor.poll to be called");

    let mut actor_ref = process.create_ref();
    actor_ref.send(())
        .expect("unable to send message");

    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(*handle_called.borrow(), 1, "expected actor.handle to be called");
    assert_eq!(*poll_called.borrow(), 1);
}

struct PollTestActor {
    result: ActorResult<()>,
}

impl Actor for PollTestActor {
    type Message = ();
    type Error = ();

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        unreachable!();
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        self.result
    }
}

struct HandleTestActor {
    result: ActorResult<()>,
}

impl Actor for HandleTestActor  {
    type Message = ();
    type Error = ();

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        self.result
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        Poll::Ready(Ok(Status::Ready))
    }
}

#[test]
#[ignore = "causes SIGILL on MacOS"]
fn actor_process_poll_statusses() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let poll_tests = vec![
        (Poll::Ready(Ok(Status::Complete)), ProcessResult::Complete),
        // Handle should return `Status::Complete`.
        (Poll::Ready(Ok(Status::Ready)), ProcessResult::Pending),
        (Poll::Ready(Err(())), ProcessResult::Complete),
        (Poll::Pending, ProcessResult::Pending),
    ];

    for test in poll_tests {
        let actor = PollTestActor { result: test.0 };
        let mut process = ActorProcess::new(ProcessId(0), actor,
            ActorOptions::default(), system_ref.clone());
        assert_eq!(process.run(&mut system_ref), test.1, "handle returned: {:?}", test.0);
    }

    let handle_tests = vec![
        (Poll::Ready(Ok(Status::Complete)), ProcessResult::Complete),
        // After the first message try to get anther one should return
        // `ProcessResult::Pending`.
        (Poll::Ready(Ok(Status::Ready)), ProcessResult::Pending),
        (Poll::Ready(Err(())), ProcessResult::Complete),
        (Poll::Pending, ProcessResult::Pending),
    ];

    for test in handle_tests {
        let actor = HandleTestActor { result: test.0 };
        let mut process = ActorProcess::new(ProcessId(0), actor,
            ActorOptions::default(), system_ref.clone());
        // Set `ready_for_msg` to true.
        assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
        process.create_ref().send(()).unwrap();
        assert_eq!(process.run(&mut system_ref), test.1, "poll returned: {:?}", test.0);
    }
}

#[test]
fn actor_ref_clone() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let mut system_ref = system.create_ref();

    let (actor, handle_called, poll_called) = SimpleActor::new();
    let mut process = ActorProcess::new(ProcessId(0), actor,
        ActorOptions::default(), system_ref.clone());
    assert_eq!(*poll_called.borrow(), 0);

    let mut actor_ref1 = process.create_ref();
    actor_ref1.send(()).expect("unable to send message");
    let mut actor_ref2 = actor_ref1.clone();
    actor_ref2.send(()).expect("unable to send message");

    if let ProcessResult::Complete = process.run(&mut system_ref) {
        panic!("expected ProcessResult::Pending");
    }
    assert_eq!(*handle_called.borrow(), 2, "expected actor.handle to be called");
}

#[test]
fn actor_ref_send_errors() {
    let system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");
    let system_ref = system.create_ref();

    let (actor, _, _) = SimpleActor::new();
    let process = ActorProcess::new(ProcessId(0), actor,
        ActorOptions::default(), system_ref);

    let mut actor_ref = process.create_ref();
    actor_ref.send(()).expect("unable to send message");

    drop(system);
    assert_eq!(actor_ref.send(()).unwrap_err(),
        SendError { message: (), reason: SendErrorReason::SystemShutdown },
        "expected an actor system shutdown message");

    drop(process);
    assert_eq!(actor_ref.send(()).unwrap_err(),
        SendError { message: (), reason: SendErrorReason::ActorShutdown },
        "expected an actor system shutdown message");
}
