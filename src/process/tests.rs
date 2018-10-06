//! Unit tests for the process module.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_channel as channel;
use mio_st::event::EventedId;
use mio_st::poll::Poller;

use crate::actor::{actor_factory, ActorContext};
use crate::initiator::Initiator;
use crate::process::{ActorProcess, InitiatorProcess, Process, ProcessId, ProcessResult};
use crate::system::ActorSystemRef;
use crate::test;
use crate::waker::new_waker;

#[test]
fn pid() {
    assert_eq!(ProcessId(0), ProcessId(0));
    assert_eq!(ProcessId(100), ProcessId(100));

    assert!(ProcessId(0) < ProcessId(100));

    assert_eq!(ProcessId(0).to_string(), "0");
    assert_eq!(ProcessId(100).to_string(), "100");
    assert_eq!(ProcessId(8000).to_string(), "8000");
}

#[test]
fn pid_and_evented_id() {
    let pid = ProcessId(0);
    let id: EventedId = pid.into();
    assert_eq!(id, EventedId(0));

    let id = EventedId(0);
    let pid: ProcessId = id.into();
    assert_eq!(pid, ProcessId(0));
}

#[derive(Debug)]
struct Error;

#[derive(Debug)]
struct Message;

async fn ok_actor(mut ctx: ActorContext<Message>, _: ()) -> Result<(), Error> {
    let _msg = await!(ctx.receive());
    Ok(())
}

#[test]
fn actor_process() {
    // Create our actor.
    let new_actor = actor_factory(ok_actor);
    let (actor, mut actor_ref) = test::init_actor(new_actor, ());

    // Create the waker.
    let pid = ProcessId(0);
    let (sender, _) = channel::unbounded();
    let waker = new_waker(pid, sender);

    // Finally create our process.
    let mut process = ActorProcess::new(actor, waker);

    // Actor should return `Poll::Pending`, because no message is ready.
    let mut system_ref = test::system_ref();
    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);

    // Send the message and the actor should return Ok.
    actor_ref.send(Message).unwrap();
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
}

async fn error_actor(_ctx: ActorContext<Message>, _: ()) -> Result<(), Error> {
    Err(Error)
}

#[test]
fn erroneous_actor_process() {
    // Create our actor.
    let new_actor = actor_factory(error_actor);
    let (actor, _) = test::init_actor(new_actor, ());

    // Create the waker.
    let pid = ProcessId(0);
    let (sender, _) = channel::unbounded();
    let waker = new_waker(pid, sender);

    // Finally create our process.
    let mut process = ActorProcess::new(actor, waker);

    // Actor should return Err.
    let mut system_ref = test::system_ref();
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
}

struct SimpleInitiator {
    called: Arc<AtomicUsize>,
}

impl Initiator for SimpleInitiator {
    fn clone_threaded(&self) -> io::Result<Self> {
        unreachable!();
    }

    fn init(&mut self, _: &mut Poller, _: ProcessId) -> io::Result<()> {
        unreachable!();
    }

    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        match self.called.fetch_add(1, Ordering::Relaxed) {
            0 => Ok(()),
            1 => Err(io::ErrorKind::Other.into()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn initiator_process() {
    let called = Arc::new(AtomicUsize::new(0));
    let mut process = InitiatorProcess::new(SimpleInitiator {
        called: Arc::clone(&called),
    });

    // Ok run.
    let mut system_ref = test::system_ref();
    assert_eq!(process.run(&mut system_ref), ProcessResult::Pending);
    assert_eq!(called.load(Ordering::Relaxed), 1);

    // Error run.
    assert_eq!(process.run(&mut system_ref), ProcessResult::Complete);
    assert_eq!(called.load(Ordering::Relaxed), 2);
}
