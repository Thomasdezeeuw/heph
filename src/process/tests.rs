//! Tests for the process module.

use std::io;
use std::cell::RefCell;
use std::future::Future;
use std::mem::PinMut;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use mio_st::event::{EventedId, Ready};
use mio_st::poll::{Poller, PollOption};
use mio_st::registration::Registration;

use crate::actor::{Actor, ActorContext, ActorResult, Status};
use crate::initiator::Initiator;
use crate::process::{ProcessId, Process, ProcessResult, ActorProcess, InitiatorProcess, TaskProcess};
use crate::system::{ActorSystemBuilder, ActorSystemRef, ActorRef, MailBox, new_waker};
use crate::util::Shared;

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

    let pid = ProcessId(0);
    let (mut registration, notifier) = Registration::new();

    let mut poller = Poller::new().unwrap();
    poller.register(&mut registration, pid.into(),
        Ready::READABLE, PollOption::Edge).unwrap();


    let waker = new_waker(notifier.clone());
    let mailbox = Shared::new(MailBox::new(notifier, system_ref.clone()));
    let mut actor_ref = ActorRef::new(mailbox.downgrade());
    let mut process = ActorProcess::new(actor, registration, waker, mailbox);

    assert_eq!(*poll_called.borrow(), 0);

    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(*poll_called.borrow(), 1, "expected actor.poll to be called");

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
    let mut system_ref = ActorSystemRef::test_ref();

    let (_, notifier) = Registration::new();
    let waker = new_waker(notifier.clone());
    let mailbox = Shared::new(MailBox::new(notifier, system_ref.clone()));
    let mut actor_ref = ActorRef::new(mailbox.downgrade());

    let poll_tests = vec![
        (Poll::Ready(Ok(Status::Complete)), ProcessResult::Complete),
        // Handle should return `Status::Complete`.
        (Poll::Ready(Ok(Status::Ready)), ProcessResult::Pending),
        (Poll::Ready(Err(())), ProcessResult::Complete),
        (Poll::Pending, ProcessResult::Pending),
    ];

    for test in poll_tests {
        let (registration, _) = Registration::new();
        let actor = PollTestActor { result: test.0 };
        let mut process = ActorProcess::new(actor, registration, waker.clone(), mailbox.clone());

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
        let (registration, _) = Registration::new();
        let actor = HandleTestActor { result: test.0 };
        let mut process = ActorProcess::new(actor, registration, waker.clone(), mailbox.clone());

        // Set `ready_for_msg` to true.
        assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
        actor_ref.send(()).unwrap();
        assert_eq!(process.run(&mut system_ref), test.1, "poll returned: {:?}", test.0);
    }
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
    let (registration, notifier) = Registration::new();
    let waker = new_waker(notifier.clone());
    let mut process = TaskProcess::new(task, registration, waker);

    // Pending run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(called.load(Ordering::SeqCst), 1, "expected the process to be run");

    // Ready run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
    assert_eq!(called.load(Ordering::SeqCst), 2, "expected the process to be run a second time");
}
