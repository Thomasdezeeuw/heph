extern crate actor;

use std::cell::RefCell;
use std::rc::Rc;

use actor::actor::{Status, actor_fn};
use actor::system::{ActorSystemBuilder, ActorOptions};
use actor::system::error::{SendError, SendErrorReason};

#[test]
fn actor_ref_clone() {
    // TODO: remove the RefCell once the actors don't have a 'static lifetime.
    let actor_value = Rc::new(RefCell::new(0));
    {
        let mut system = ActorSystemBuilder::default().build()
            .expect("can't build actor system");

        let actor_value = actor_value.clone();
        let actor = actor_fn(move |_, value: i32| -> Result<Status, ()> {
            *actor_value.borrow_mut() += value;
            Ok(Status::Ready)
        });

        let mut actor_ref1 = system.add_actor(actor, ActorOptions::default());
        actor_ref1.send(2).expect("unable to send message");

        let mut actor_ref2 = actor_ref1.clone();
        actor_ref2.send(4).expect("unable to send message");

        assert!(system.run().is_ok());
    }

    assert_eq!(*actor_value.borrow(), 6, "expected actor to be run");
}

#[test]
fn actor_ref_send_error() {
    let mut system = ActorSystemBuilder::default().build()
        .expect("can't build actor system");

    let actor1 = actor_fn(|_, _: ()| -> Result<Status, ()> {
        Ok(Status::Complete)
    });

    let mut actor1_ref = system.add_actor(actor1, ActorOptions::default());

    let mut state = 0;
    let actor2 = actor_fn(move |_, _: ()| -> Result<Status, ()> {
        state += 1;
        match state {
            1 => {
                // First message should stop the actor.
                actor1_ref.send(()).expect("unable to send message");
            },
            2 => {
                // Now the actor is shutdown we should receive an error.
                assert_eq!(actor1_ref.send(()).unwrap_err(),
                    SendError { message: (), reason: SendErrorReason::ActorShutdown },
                    "expected an actor system shutdown message");
            },
            _ => unreachable!(),
        }

        Ok(Status::Ready)
    });

    let mut actor2_ref = system.add_actor(actor2, ActorOptions::default());
    actor2_ref.send(()).expect("unable to send message");
    actor2_ref.send(()).expect("unable to send message");
}
