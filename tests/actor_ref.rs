#![feature(async_await, await_macro, futures_api, never_type)]

use std::cell::RefCell;
use std::rc::Rc;

use heph::actor::{ActorContext, actor_factory};
use heph::actor_ref::LocalActorRef;
use heph::error::{SendError, ErrorReason};
use heph::system::{ActorSystemBuilder, ActorOptions};

async fn count_actor(mut ctx: ActorContext<i32>, total: Rc<RefCell<i32>>) -> Result<(), !> {
    loop {
        let value = await!(ctx.receive());
        *total.borrow_mut() += value;
    }
}

#[test]
fn actor_ref_clone() {
    // TODO: remove the RefCell once the actors don't have a 'static lifetime.
    let total = Rc::new(RefCell::new(0));
    {
        let mut system = ActorSystemBuilder::default().build()
            .expect("can't build actor system");

        let new_actor = actor_factory(count_actor);
        let mut actor_ref1 = system.add_actor(new_actor, total.clone(),
            ActorOptions::default());

        actor_ref1.send(2).expect("unable to send message");

        let mut actor_ref2 = actor_ref1.clone();
        actor_ref2.send(4).expect("unable to send message");

        assert!(system.run().is_ok());
    }

    assert_eq!(*total.borrow(), 6, "expected actor to be run");
}

async fn single_receive_actor(mut ctx: ActorContext<()>, _: ()) -> Result<(), !> {
    let _ = await!(ctx.receive());
    Ok(())
}

async fn sending_actor(_ctx: ActorContext<()>, mut actor_ref: LocalActorRef<()>) -> Result<(), !> {
    actor_ref.send(()).expect("unable to send first message");
    assert_eq!(actor_ref.send(()), Err(SendError { message: (), reason: ErrorReason::ActorShutdown }));
    Ok(())
}

#[test]
fn actor_ref_send_error() {
    let mut _actor_ref = {
        let mut system = ActorSystemBuilder::default().build()
            .expect("can't build actor system");

        let actor1 = actor_factory(single_receive_actor);
        let actor1_ref = system.add_actor(actor1, (), ActorOptions::default());

        let actor2 = actor_factory(sending_actor);
        let actor2_ref = system.add_actor(actor2, actor1_ref, ActorOptions::default());

        system.run().expect("unexpected error running actor system");

        actor2_ref
    };

    // FIXME: currently this returns ActorShutdown, which is technically correct
    // but not what we want.
    //assert_eq!(actor_ref.send(()), Err(SendError { message: (), reason: ErrorReason::SystemShutdown }));
}
