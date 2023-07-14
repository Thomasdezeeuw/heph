//! Tests for the `test` module.

use std::mem::size_of;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::test::{size_of_actor, size_of_actor_val};

#[test]
fn test_size_of_actor() {
    async fn actor1(_: actor::Context<!, ()>) {
        /* Nothing. */
    }

    assert_eq!(size_of_actor_val(&actor_fn(actor1)), 24);

    struct Na;

    impl NewActor for Na {
        type Message = !;
        type Argument = ();
        type Actor = A;
        type Error = !;
        type RuntimeAccess = ();

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::RuntimeAccess>,
            _: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            Ok(A)
        }
    }

    struct A;

    impl Actor for A {
        type Error = !;
        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    assert_eq!(size_of::<A>(), 0);
    assert_eq!(size_of_actor::<Na>(), 0);
    assert_eq!(size_of_actor_val(&Na), 0);
}
