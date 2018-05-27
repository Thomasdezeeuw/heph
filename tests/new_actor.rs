extern crate actor;
extern crate futures_core;

use actor::actor::{Actor, NewActor, actor_factory, reusable_actor_factory};
use futures_core::Async;
use futures_core::future::{FutureResult, ok};

mod util;

use util::quick_poll;

struct SimpleActor;

impl<'a> Actor<'a> for SimpleActor {
    type Message = ();
    type Error = ();
    type Future = FutureResult<(), Self::Error>;
    fn handle(&'a mut self, _: Self::Message) -> Self::Future {
        ok(())
    }
}

struct SimpleNewActor;

impl<'n, 'a> NewActor<'n, 'a> for SimpleNewActor {
    type Actor = SimpleActor;
    type Item = ();
    fn new(&'n mut self, _: Self::Item) -> Self::Actor {
        SimpleActor
    }
}


#[test]
fn actor_does_not_need_new_actor_to_life() {
    let mut actor = {
        // The `NewActor` get dropped after this scope, but the returned actor
        // should still be usable.
        let mut new_actor = SimpleNewActor;
        new_actor.new(())
    };

    let mut future = actor.handle(());
    match quick_poll(&mut future) {
        Ok(Async::Ready(())) =>{},
        _ => panic!("expected the future to be ready, but isn't"),
    }
}

struct TestActor {
    handle_call_count: usize,
    reset_called_count: usize,
}

impl TestActor {
    fn new() -> TestActor {
        TestActor {
            handle_call_count: 0,
            reset_called_count: 0,
        }
    }

    fn reset(&mut self) {
        self.reset_called_count += 1;
    }
}

impl<'a> Actor<'a> for TestActor {
    type Message = ();
    type Error = ();
    type Future = FutureResult<(), ()>;
    fn handle(&mut self, _: ()) -> Self::Future {
        self.handle_call_count += 1;
        ok(())
    }
}

#[test]
fn test_actor_factory() {
    let mut called_new_count = 0;
    let actor = {
        let mut factory = actor_factory(|_: ()| {
            called_new_count += 1;
            TestActor::new()
        });

        let mut actor = factory.new(());
        let mut future = actor.handle(());
        assert_eq!(quick_poll(&mut future), Ok(Async::Ready(())));

        factory.reuse(&mut actor, ());
        let mut future = actor.handle(());
        assert_eq!(quick_poll(&mut future), Ok(Async::Ready(())));
        actor
    };

    assert_eq!(called_new_count, 2);
    assert_eq!(actor.handle_call_count, 1);
    assert_eq!(actor.reset_called_count, 0);
}

#[test]
fn test_actor_reuse_factory() {
    let mut called_new_count = 0;
    let actor = {
        let mut factory = reusable_actor_factory(|_: ()| {
            called_new_count += 1;
            TestActor::new()
        }, |actor: &mut TestActor, _| actor.reset());


        let mut actor = factory.new(());
        let mut future = actor.handle(());
        assert_eq!(quick_poll(&mut future), Ok(Async::Ready(())));

        factory.reuse(&mut actor, ());
        let mut future = actor.handle(());
        assert_eq!(quick_poll(&mut future), Ok(Async::Ready(())));
        actor
    };

    assert_eq!(called_new_count, 1);
    assert_eq!(actor.handle_call_count, 2);
    assert_eq!(actor.reset_called_count, 1);
}
