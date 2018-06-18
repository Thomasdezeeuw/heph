//! Tests for the `system` module.

use std::mem;
use std::cell::RefCell;
use std::ops::AddAssign;
use std::rc::Rc;

use actor::actor::{Actor, actor_fn};
use actor::system::{ActorSystemBuilder, ActorOptions};
use futures_core::task::Context;
use futures_core::{Future, Async, Poll};

#[test]
fn no_initiator_single_message_before_run() {
    let actor_value = Rc::new(RefCell::new(0));
    {
        let mut actor_system = ActorSystemBuilder::default().build()
            .expect("unable to build actor system");

        let actor_value = Rc::clone(&actor_value);
        let actor = actor_fn(move |value: usize| -> Result<(), ()> {
            actor_value.borrow_mut().add_assign(value);
            Ok(())
        });

        let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
            .expect("unable to add actor to actor system");

        actor_ref.send(1usize).expect("failed to send to actor");

        actor_system.run()
            .expect("unable to run actor system");
    }
    assert_eq!(*actor_value.borrow(), 1, "actor didn't receive message");
}

#[test]
fn no_initiator_multiple_messages_before_run() {
    let actor_value = Rc::new(RefCell::new(0));
    {
        let mut actor_system = ActorSystemBuilder::default().build()
            .expect("unable to build actor system");

        let actor_value = Rc::clone(&actor_value);
        let actor = actor_fn(move |value: usize| -> Result<(), ()> {
            actor_value.borrow_mut().add_assign(value);
            Ok(())
        });

        let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
            .expect("unable to add actor to actor system");

        actor_ref.send(1usize).expect("failed to send to actor");
        actor_ref.send(2usize).expect("failed to send to actor");
        actor_ref.send(4usize).expect("failed to send to actor");

        actor_system.run()
            .expect("unable to run actor system");
    }
    assert_eq!(*actor_value.borrow(), 7, "actor didn't receive message");
}

#[test]
fn no_initiator_run() {
    let actor1_value = Rc::new(RefCell::new(0));
    let actor2_value = Rc::new(RefCell::new(0));
    {
        let mut actor_system = ActorSystemBuilder::default().build()
            .expect("unable to build actor system");

        let actor1_value = Rc::clone(&actor1_value);
        let actor1 = actor_fn(move |value: usize| -> Result<(), ()> {
            actor1_value.borrow_mut().add_assign(value);
            Ok(())
        });
        let mut actor1_ref = actor_system.add_actor(actor1, ActorOptions::default())
            .expect("unable to add actor to actor system");

        let actor2_value = Rc::clone(&actor2_value);
        let actor2 = actor_fn(move |value: usize| -> Result<(), ()> {
            actor2_value.borrow_mut().add_assign(value);
            actor1_ref.send(value + 1).expect("failed to send to actor");
            Ok(())
        });
        let mut actor2_ref = actor_system.add_actor(actor2, ActorOptions::default())
            .expect("unable to add actor to actor system");

        actor2_ref.send(1usize).expect("failed to send to actor");
        actor2_ref.send(2usize).expect("failed to send to actor");
        actor2_ref.send(4usize).expect("failed to send to actor");

        actor_system.run()
            .expect("unable to run actor system");
    }
    assert_eq!(*actor1_value.borrow(), 10, "actor1 didn't receive message");
    assert_eq!(*actor2_value.borrow(), 7, "actor2 didn't receive message");
}

#[derive(Debug)]
enum TestLifetimeActor {
    /// Just started.
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

impl Future for TestLifetimeActor {
    type Item = ();
    type Error = ();
    fn poll(&mut self, _: &mut Context) -> Poll<Self::Item, Self::Error> {
        use self::TestLifetimeActor::*;
        const ERR: &str = "called poll before returning Async::Pending";
        match self {
            Init => panic!("called poll before handle"),
            RecvMsg => {
                mem::replace(self, TestLifetimeActor::MsgHandled);
                Ok(Async::Ready(()))
            },
            MsgHandled => panic!(ERR),
            MsgHandled2 => panic!(ERR),
            ReturnedError => panic!(ERR),
        }
    }
}

impl Actor for TestLifetimeActor {
    type Message = ();
    fn handle(&mut self, ctx: &mut Context, _: Self::Message) -> Poll<Self::Item, Self::Error> {
        use self::TestLifetimeActor::*;
        match self {
            Init => {
                mem::replace(self, TestLifetimeActor::RecvMsg);
                ctx.waker().wake();
                Ok(Async::Pending)
            },
            RecvMsg => panic!("called handle after returning Async::Pending"),
            MsgHandled => {
                mem::replace(self, TestLifetimeActor::MsgHandled2);
                Ok(Async::Ready(()))
            },
            MsgHandled2 => {
                mem::replace(self, TestLifetimeActor::ReturnedError);
                Err(())
            },
            ReturnedError => panic!("called handle after returning an error"),
        }
    }
}

#[test]
fn simple_actor_lifetime_test() {
    let actor = TestLifetimeActor::Init;

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
