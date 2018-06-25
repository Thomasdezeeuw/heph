//! Tests for the `actor` module.

use std::task::Poll;

use actor::actor::{NewActor, Status, actor_fn, actor_factory, reusable_actor_factory};

use util::{TestActor, TestMessage, quick_handle};

#[test]
fn test_actor_fn() {
    let mut actor_value = 0;
    {
        let mut returned_ok = false;
        let mut actor = actor_fn(|_, value: usize| {
            actor_value += value;
            if !returned_ok {
                returned_ok = true;
                Ok(Status::Ready)
            } else {
                Err(())
            }
        });

        assert_eq!(quick_handle(&mut actor, 1), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(quick_handle(&mut actor, 10), Poll::Ready(Err(())));
    }
    assert_eq!(actor_value, 11);
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
        assert_eq!(quick_handle(&mut actor, TestMessage(2)), Poll::Ready(Ok(Status::Ready)));

        factory.reuse(&mut actor, ());
        assert_eq!(quick_handle(&mut actor, TestMessage(4)), Poll::Ready(Ok(Status::Ready)));
        actor
    };

    assert_eq!(called_new_count, 2);
    assert_eq!(actor.value, 4);
    assert_eq!(actor.reset, 0);
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
        assert_eq!(quick_handle(&mut actor, TestMessage(2)), Poll::Ready(Ok(Status::Ready)));

        factory.reuse(&mut actor, ());
        assert_eq!(quick_handle(&mut actor, TestMessage(4)), Poll::Ready(Ok(Status::Ready)));
        actor
    };

    assert_eq!(called_new_count, 1);
    assert_eq!(actor.value, 4);
    assert_eq!(actor.reset, 1);
}
