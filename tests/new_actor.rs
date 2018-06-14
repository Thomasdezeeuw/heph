extern crate actor;
extern crate futures_core;

use actor::actor::{NewActor, actor_factory, reusable_actor_factory};
use futures_core::Async;

mod util;

use util::{TestActor, TestMessage, quick_handle};

#[test]
fn test_actor_factory() {
    let mut called_new_count = 0;
    let actor = {
        let mut factory = actor_factory(|_: ()| {
            called_new_count += 1;
            TestActor::new()
        });

        let mut actor = factory.new(());
        assert_eq!(quick_handle(&mut actor, TestMessage(2)), Ok(Async::Ready(())));

        factory.reuse(&mut actor, ());
        assert_eq!(quick_handle(&mut actor, TestMessage(4)), Ok(Async::Ready(())));
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
        assert_eq!(quick_handle(&mut actor, TestMessage(2)), Ok(Async::Ready(())));

        factory.reuse(&mut actor, ());
        assert_eq!(quick_handle(&mut actor, TestMessage(4)), Ok(Async::Ready(())));
        actor
    };

    assert_eq!(called_new_count, 1);
    assert_eq!(actor.value, 4);
    assert_eq!(actor.reset, 1);
}
