use std::task::Poll;

use crate::actor::{Actor, ActorContext, Status, actor_fn};

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
