//! Tests for the `system` module.

use std::fmt;
use std::cell::RefCell;
use std::ops::AddAssign;
use std::rc::Rc;

use actor::actor::{Actor, Status, actor_fn};
use actor::system::{ActorSystemBuilder, ActorOptions};

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

/* FIXME: able this.
// TODO: add stage for returning `Shutdown::Ready`.
#[derive(Debug)]
enum TestLifetimeActor {
    /// Initial status.
    Init,
    /// Received a message, returned `Async::Pending` in handle..
    RecvMsg,
    /// Handled the first message, returned `Async::Ready` in poll.
    MsgHandled,
    /// Received another message, returned `Async::Ready` in handle.
    MsgHandled2,
    /// Received a third message, returned `Err` in handle.
    ReturnedError,
}

impl Actor for TestLifetimeActor {
    type Message = Shutdown;
    type Error = ();

    fn handle(&mut self, ctx: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        use self::TestLifetimeActor::*;
        match self {
            Init => {
                mem::replace(self, TestLifetimeActor::RecvMsg);
                ctx.task_ctx().waker().wake();
                Ok(Async::Pending)
            },
            RecvMsg => panic!("called handle after returning Async::Pending"),
            MsgHandled => {
                mem::replace(self, TestLifetimeActor::MsgHandled2);
                Ok(Async::Ready(Shutdown::Pending))
            },
            MsgHandled2 => {
                mem::replace(self, TestLifetimeActor::ReturnedError);
                Err(())
            },
            ReturnedError => panic!("called handle after returning an error"),
        }
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        use self::TestLifetimeActor::*;
        const ERR: &str = "called poll before returning Async::Pending";
        match self {
            Init => panic!("called poll before handle"),
            RecvMsg => {
                mem::replace(self, TestLifetimeActor::MsgHandled);
                Ok(Async::Ready(Shutdown::Pending))
            },
            MsgHandled => panic!(ERR),
            MsgHandled2 => panic!(ERR),
            ReturnedError => panic!(ERR),
        }
    }
}

#[test]
fn simple_actor_lifetime_test() {
    let status = TestLifetimeActor::Init;

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");

    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
        .expect("unable to add actor to actor system");

    actor_ref.send(()).expect("failed to send to actor");
    actor_ref.send(()).expect("failed to send to actor");
    actor_ref.send(()).expect("failed to send to actor");
    // Next message shouldn't arrive.
    actor_ref.send(()).expect("failed to send to actor");

    actor_system.run()
        .expect("unable to run actor system");
}
*/

/*
#[derive(Debug)]
enum TestSendActor1 {
    // Wait for message from start.
    Init,
    // Send message 1.
    Send1,
    // Send message 1.
    Send2,
    Done,
}

impl Future for TestSendActor1 {
    type Item = ();
    type Error = ();
    fn poll(&mut self, _: &mut Context) -> Poll<Self::Item, Self::Error> {
        use self::TestSendActor1::*;
        const ERR: &str = "called poll before returning Async::Pending";
        match self {
            Init => panic!("called poll before handle"),
            RecvMsg => {
                mem::replace(self, TestSendActor1::MsgHandled);
                Ok(Async::Ready(()))
            },
            MsgHandled => panic!(ERR),
            MsgHandled2 => panic!(ERR),
            ReturnedError => panic!(ERR),
        }
    }
}

impl Actor for TestSendActor1 {
    type Message = ();
    fn handle(&mut self, ctx: &mut Context, _: Self::Message) -> Poll<Self::Item, Self::Error> {
        use self::TestSendActor1::*;
        match self {
            Init => {
                mem::replace(self, TestSendActor1::RecvMsg);
                ctx.waker().wake();
                Ok(Async::Pending)
            },
            RecvMsg => panic!("called handle after returning Async::Pending"),
            MsgHandled => {
                mem::replace(self, TestSendActor1::MsgHandled2);
                Ok(Async::Ready(()))
            },
            MsgHandled2 => {
                mem::replace(self, TestSendActor1::ReturnedError);
                Err(())
            },
            ReturnedError => panic!("called handle after returning an error"),
        }
    }
}

#[derive(Debug)]
enum TestSendActor2 {
    // Wait for message from actor 1.
    Init,
    // Send message 1.
    Send1,
    // Send message 2 and we're done.
    Done,
}

impl Future for TestSendActor2 {
    type Item = ();
    type Error = ();
    fn poll(&mut self, _: &mut Context) -> Poll<Self::Item, Self::Error> {
        use self::TestSendActor2::*;
        const ERR: &str = "called poll before returning Async::Pending";
        match self {
            Init => panic!("called poll before handle"),
            RecvMsg => {
                mem::replace(self, TestSendActor2::MsgHandled);
                Ok(Async::Ready(()))
            },
            MsgHandled => panic!(ERR),
            MsgHandled2 => panic!(ERR),
            ReturnedError => panic!(ERR),
        }
    }
}

impl Actor for TestSendActor2 {
    type Message = ();
    fn handle(&mut self, ctx: &mut Context, _: Self::Message) -> Poll<Self::Item, Self::Error> {
        use self::TestSendActor2::*;
        match self {
            Init => {
                mem::replace(self, TestSendActor2::RecvMsg);
                ctx.waker().wake();
                Ok(Async::Pending)
            },
            RecvMsg => panic!("called handle after returning Async::Pending"),
            MsgHandled => {
                mem::replace(self, TestSendActor2::MsgHandled2);
                Ok(Async::Ready(()))
            },
            MsgHandled2 => {
                mem::replace(self, TestSendActor2::ReturnedError);
                Err(())
            },
            ReturnedError => panic!("called handle after returning an error"),
        }
    }
}

#[test]
fn two_actors_test() {
    let actor1 = TestSendActor1::Init;
    let actor2 = TestSendActor2::Init;

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");

    let mut actor1_ref = actor_system.add_actor(actor1, ActorOptions::default())
        .expect("unable to add actor to actor system");
    actor_system.add_actor(actor2, ActorOptions::default())
        .expect("unable to add actor to actor system");

    actor1_ref.send(()).expect("failed to send to actor");

    actor_system.run()
        .expect("unable to run actor system");
}
*/
