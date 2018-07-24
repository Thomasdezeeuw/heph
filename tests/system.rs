#![feature(futures_api)]

extern crate actor;

use std::{fmt, mem};
use std::cell::RefCell;
use std::ops::AddAssign;
use std::rc::Rc;
use std::task::Poll;

use actor::actor::{Actor, ActorContext, ActorResult, Status, actor_fn};
use actor::system::{ActorSystemBuilder, ActorRef, ActorOptions};

/// Little helper trait to call expect on the result of `ActorSystem.add_actor`.
trait DebugActor: Actor + fmt::Debug {}

impl<T: Actor + fmt::Debug> DebugActor for T {}

fn new_count_actor() -> (impl DebugActor<Message = usize, Error = ()>, Rc<RefCell<usize>>) {
    let value = Rc::new(RefCell::new(0));
    let actor_value = Rc::clone(&value);
    let actor = actor_fn(move |_, value: usize| -> Result<Status, ()> {
        actor_value.borrow_mut().add_assign(value);
        Ok(Status::Ready)
    });
    (actor, value)
}

#[test]
fn no_initiator_single_message_before_run() {
    let (actor, value) = new_count_actor();

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    actor_ref.send(1usize).expect("failed to send to actor");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*value.borrow(), 1, "actor didn't receive message");
}

#[test]
fn no_initiator_multiple_messages_before_run() {
    let (actor, value) = new_count_actor();

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    actor_ref.send(1usize).expect("failed to send to actor");
    actor_ref.send(2usize).expect("failed to send to actor");
    actor_ref.send(4usize).expect("failed to send to actor");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*value.borrow(), 7, "actor didn't receive messages");
}

#[test]
fn simple_message_passing() {
    let (actor1, value) = new_count_actor();

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");
    let mut actor1_ref = actor_system.add_actor(actor1, ActorOptions::default());

    let actor2 = actor_fn(move |_, value: usize| -> Result<Status, ()> {
        actor1_ref.send(value * 2).expect("failed to send to actor");
        Ok(Status::Ready)
    });

    let mut actor2_ref = actor_system.add_actor(actor2, ActorOptions::default());

    actor2_ref.send(1usize).expect("failed to send to actor");
    actor2_ref.send(2usize).expect("failed to send to actor");
    actor2_ref.send(4usize).expect("failed to send to actor");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*value.borrow(), 14, "actor2 didn't receive messages");
}

#[derive(Debug)]
enum TestLifetimeActor {
    /// Initial status (handled in poll)
    Init,
    /// After first call to poll, ready for first message (handled in handle).
    ReadyMsg1,
    /// Received first message, returned `Poll::Pending` in handle and called
    /// waker (poll).
    RecvMsg1,
    /// Second call to poll, completely handled the first message.
    /// Ready to receive the second message (handle).
    ReadyMsg2,
    /// Handled second message, returned `Poll::Ready` (handle).
    ReadyMsg3,
    /// Handled third message, returned an error (handle).
    ReturnedError,
}

impl Actor for TestLifetimeActor {
    type Message = ();
    type Error = ();

    fn handle(&mut self, ctx: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        use self::TestLifetimeActor::*;
        match self {
            ReadyMsg1 => {
                mem::replace(self, RecvMsg1);
                ctx.task_ctx().waker().wake();
                Poll::Pending
            },
            ReadyMsg2 => {
                mem::replace(self, ReadyMsg3);
                Poll::Ready(Ok(Status::Ready))
            },
            ReadyMsg3 => {
                mem::replace(self, ReturnedError);
                Poll::Ready(Err(()))
            },
            state => panic!("unexpectedly called handle in {:?} state", state),
        }
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        use self::TestLifetimeActor::*;
        match self {
            Init => {
                mem::replace(self, ReadyMsg1);
                Poll::Ready(Ok(Status::Ready))
            },
            RecvMsg1 => {
                mem::replace(self, ReadyMsg2);
                Poll::Ready(Ok(Status::Ready))
            },
            state => panic!("unexpectedly called poll in {:?} state", state),
        }
    }
}

#[test]
fn simple_actor_lifetime_test() {
    let actor = TestLifetimeActor::Init;

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");

    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    actor_ref.send(()).expect("failed to send to actor");
    actor_ref.send(()).expect("failed to send to actor");
    actor_ref.send(()).expect("failed to send to actor");
    // Next message shouldn't arrive.
    actor_ref.send(()).expect("failed to send to actor");

    actor_system.run()
        .expect("unable to run actor system");
}

#[derive(Debug)]
enum TestSendActor1 {
    /// Initial state (handled in poll).
    Init,
    /// Ready to receive first message (handled in handle).
    ReadyMsg1,
    /// Ready to send the second message (handled in poll).
    SendMsg2(ActorRef<()>),
    /// All done.
    Done,
}

impl Actor for TestSendActor1 {
    type Message = ActorRef<()>;
    type Error = ();

    fn handle(&mut self, ctx: &mut ActorContext, mut msg: Self::Message) -> ActorResult<Self::Error> {
        use self::TestSendActor1::*;
        match self {
            ReadyMsg1 => {
                mem::replace(self, SendMsg2(msg.clone()));
                msg.send(()).expect("unable to send message to actor 2");
                ctx.task_ctx().waker().wake();
                Poll::Pending
            },
            state => panic!("unexpectedly called TestSendActor1.handle in {:?} state", state),
        }
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        use self::TestSendActor1::*;
        match self {
            Init => {
                mem::replace(self, ReadyMsg1);
                return Poll::Ready(Ok(Status::Ready));
            },
            SendMsg2(ref mut actor_ref) => {
                actor_ref.send(()).expect("unable to send message to actor 2");
            },
            state => panic!("unexpectedly called TestSendActor1.poll in {:?} state", state),
        }

        // After SendMsg2.
        mem::replace(self, Done);
        Poll::Ready(Ok(Status::Complete))
    }
}

#[derive(Debug)]
enum TestSendActor2 {
    /// Initial state (handled in poll).
    Init,
    /// Ready to receive the first message (handled in handle).
    ReadyMsg1,
    /// Ready to receive the second message (handled in handle).
    ReadyMsg2,
    /// Ready to receive the third message (handled in handle).
    ReadyMsg3,
    /// All done.
    Done,
}

impl Actor for TestSendActor2 {
    type Message = ();
    type Error = ();

    fn handle(&mut self, _ctx: &mut ActorContext, _msg: Self::Message) -> ActorResult<Self::Error> {
        use self::TestSendActor2::*;
        match self {
            ReadyMsg1 => {
                mem::replace(self, ReadyMsg2);
                Poll::Ready(Ok(Status::Ready))
            },
            ReadyMsg2 => {
                mem::replace(self, ReadyMsg3);
                Poll::Ready(Ok(Status::Ready))
            },
            ReadyMsg3 => {
                mem::replace(self, Done);
                Poll::Ready(Ok(Status::Complete))
            },
            state => panic!("unexpectedly called TestSendActor2.handle in {:?} state", state),
        }
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        use self::TestSendActor2::*;
        match self {
            Init => {
                mem::replace(self, ReadyMsg1);
                Poll::Ready(Ok(Status::Ready))
            },
            state => panic!("unexpectedly called TestSendActor2.poll in {:?} state", state),
        }
    }
}

#[test]
fn two_actors_test() {
    let actor1 = TestSendActor1::Init;
    let actor2 = TestSendActor2::Init;

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");

    let mut actor1_ref = actor_system.add_actor(actor1, ActorOptions::default());
    let mut actor2_ref = actor_system.add_actor(actor2, ActorOptions::default());

    actor2_ref.send(()).expect("failed to send to actor2");
    actor1_ref.send(actor2_ref).expect("failed to send to actor1");

    actor_system.run().expect("unable to run actor system");
}
