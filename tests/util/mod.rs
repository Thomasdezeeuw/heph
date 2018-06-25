//! Module with utilities for testing.

use std::task::Poll;

use actor::actor::{Actor, ActorContext, ActorResult, Status};

/// Quickly call `handle` on the actor with the provided `msg`.
pub fn quick_handle<A: Actor>(actor: &mut A, msg: A::Message) -> ActorResult<A::Error> {
    let mut ctx = ActorContext{};
    actor.handle(&mut ctx, msg)
}

/// Simple testing actor.
pub struct TestActor {
    pub value: usize,
    /// Reset counter.
    pub reset: usize,
}

impl TestActor {
    pub fn new() -> TestActor {
        TestActor { value: 0, reset: 0 }
    }

    pub fn reset(&mut self) {
        self.value = 0;
        self.reset += 1;
    }
}

/// Increase the internal value of an `TestActor`.
pub struct TestMessage(pub usize);

impl Actor for TestActor {
    type Message = TestMessage;
    type Error = ();

    fn handle(&mut self, _: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error> {
        self.value += msg.0;
        Poll::Ready(Ok(Status::Ready))
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        Poll::Ready(Ok(Status::Ready))
    }
}
