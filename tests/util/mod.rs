#![allow(dead_code)]

use std::sync::Arc;

use actor::actor::Actor;
use futures_core::task::{Context, LocalMap, Waker, Wake};
use futures_core::{Future, Async, Poll};

/// Simple `Waker` implementation that does nothing.
struct NopWaker;

impl Wake for NopWaker {
    fn wake(_: &Arc<Self>) { }
}

/// Quickly call `handle` on the actor with the provided `msg`.
pub fn quick_handle<A: Actor>(actor: &mut A, msg: A::Message) -> Poll<(), A::Error> {
    let mut map = LocalMap::new();
    let mut waker = Waker::from(Arc::new(NopWaker));
    let mut ctx = Context::without_spawn(&mut map, &mut waker);
    actor.handle(&mut ctx, msg)
}

/// Quickly poll the `future`, with an empty map, no-op waker and without a
/// spawner.
pub fn quick_poll<F: Future>(future: &mut F) -> Poll<F::Item, F::Error> {
    let mut map = LocalMap::new();
    let mut waker = Waker::from(Arc::new(NopWaker));
    let mut ctx = Context::without_spawn(&mut map, &mut waker);
    future.poll(&mut ctx)
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

impl Future for TestActor {
    type Item = ();
    type Error = ();
    fn poll(&mut self, _: &mut Context) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(()))
    }
}

impl Actor for TestActor {
    type Message = TestMessage;
    fn handle(&mut self, _: &mut Context, msg: Self::Message) -> Poll<(), Self::Error> {
        self.value += msg.0;
        Ok(Async::Ready(()))
    }
}
