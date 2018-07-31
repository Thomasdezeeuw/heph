#![feature(async_await, await_macro, futures_api, never_type)]

extern crate actor;

use std::cell::RefCell;
use std::rc::Rc;

use actor::actor::{ActorContext, actor_factory};
use actor::system::{ActorRef, ActorSystemBuilder, ActorOptions};

async fn count_actor(mut ctx: ActorContext<i32>, total: Rc<RefCell<i32>>) -> Result<(), !> {
    loop {
        let value = await!(ctx.receive());
        *total.borrow_mut() += value;
    }
}

#[test]
fn no_initiator_single_message_before_run() {
    let total = Rc::new(RefCell::new(0));

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");

    let new_actor = actor_factory(count_actor);
    let mut actor_ref = actor_system.add_actor(new_actor, total.clone(),
        ActorOptions::default());

    actor_ref.send(1).expect("unable to send message");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*total.borrow(), 1, "expected actor to be run");
}

#[test]
fn no_initiator_multiple_messages_before_run() {
    let total = Rc::new(RefCell::new(0));

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");

    let new_actor = actor_factory(count_actor);
    let mut actor_ref = actor_system.add_actor(new_actor, total.clone(),
        ActorOptions::default());

    actor_ref.send(1).expect("unable to send message");
    actor_ref.send(2).expect("unable to send message");
    actor_ref.send(3).expect("unable to send message");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*total.borrow(), 6, "expected actor to be run");
}

async fn relay_actor(mut ctx: ActorContext<i32>, mut actor_ref: ActorRef<i32>) -> Result<(), !> {
    loop {
        let value = await!(ctx.receive());
        actor_ref.send(value).expect("unable to relay message");
    }
}

#[test]
fn simple_message_passing() {
    let total = Rc::new(RefCell::new(0));

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build actor system");

    let new_actor1 = actor_factory(count_actor);
    let actor1_ref = actor_system.add_actor(new_actor1, total.clone(),
        ActorOptions::default());

    let new_actor2 = actor_factory(relay_actor);
    let mut actor2_ref = actor_system.add_actor(new_actor2, actor1_ref,
        ActorOptions::default());

    actor2_ref.send(1).expect("failed to send to actor");
    actor2_ref.send(2).expect("failed to send to actor");
    actor2_ref.send(4).expect("failed to send to actor");

    actor_system.run().expect("unable to run actor system");

    assert_eq!(*total.borrow(), 7, "expected actor to be run");
}
