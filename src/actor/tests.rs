use std::task::Poll;

use crate::actor::{Actor, NewActor, ActorContext, ActorResult, Status, actor_fn, actor_factory, reusable_actor_factory};

#[test]
fn test_actor_fn() {
    let mut actor_value = 0;
    {
        let mut returned_ok = false;
        let mut actor = actor_fn(|_ctx, value: usize| {
            actor_value += value;
            if !returned_ok {
                returned_ok = true;
                Ok(Status::Ready)
            } else {
                Err(())
            }
        });

        let mut ctx = ActorContext::test_ctx();
        assert_eq!(actor.handle(&mut ctx, 1), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(actor.handle(&mut ctx, 10), Poll::Ready(Err(())));

        assert_eq!(actor.poll(&mut ctx), Poll::Ready(Ok(Status::Ready)));
    }
    assert_eq!(actor_value, 11);
}

/// Simple testing actor.
pub struct TestActor {
    pub value: usize,
    /// Reset counter.
    pub reset: usize,
}

impl TestActor {
    pub const fn new() -> TestActor {
        TestActor { value: 0, reset: 0 }
    }

    pub fn reset(&mut self) {
        self.value = 0;
        self.reset += 1;
    }
}

impl Actor for TestActor {
    type Message = usize;
    type Error = ();

    fn handle(&mut self, _: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error> {
        self.value += msg;
        Poll::Ready(Ok(Status::Ready))
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        Poll::Ready(Ok(Status::Ready))
    }
}

#[test]
fn test_actor_factory() {
    let mut called_new_count = 0;
    {
        let mut factory = actor_factory(|_: ()| {
            called_new_count += 1;
            TestActor::new()
        });

        let mut actor = factory.new(());
        let mut ctx = ActorContext::test_ctx();
        assert_eq!(actor.handle(&mut ctx, 1), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(actor.value, 1);
        assert_eq!(actor.reset, 0);

        factory.reuse(&mut actor, ());
        assert_eq!(actor.handle(&mut ctx, 2), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(actor.value, 2);
        assert_eq!(actor.reset, 0);
    };

    assert_eq!(called_new_count, 2);
}

#[test]
fn test_actor_reuse_factory() {
    let mut called_new_count = 0;
    {
        let mut factory = reusable_actor_factory(|_: ()| {
            called_new_count += 1;
            TestActor::new()
        }, |actor, _| actor.reset());

        let mut actor = factory.new(());
        let mut ctx = ActorContext::test_ctx();
        assert_eq!(actor.handle(&mut ctx, 1), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(actor.value, 1);
        assert_eq!(actor.reset, 0);

        factory.reuse(&mut actor, ());
        assert_eq!(actor.handle(&mut ctx, 2), Poll::Ready(Ok(Status::Ready)));
        assert_eq!(actor.value, 2);
        assert_eq!(actor.reset, 1);
    };

    assert_eq!(called_new_count, 1);
}
