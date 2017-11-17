// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

extern crate futures;

extern crate actor;

use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::{Future, FutureResult, ok};

use actor::actor::{Actor, NewActor, ActorFactory, ActorReuseFactory};

struct TestActor {
    handle_call_count: usize,
}

impl TestActor {
    fn new() -> TestActor {
        TestActor {
            handle_call_count: 0,
        }
    }
}

impl Actor for TestActor {
    type Message = ();
    type Error = ();
    type Future = FutureResult<(), ()>;
    fn handle(&mut self, _: ()) -> Self::Future {
        self.handle_call_count += 1;
        ok(())
    }
}

#[test]
fn actor_factory() {
    let called_new_count = AtomicUsize::new(0);
    let new_actor = ActorFactory::new(|| {
        called_new_count.fetch_add(1, Ordering::Relaxed);
        TestActor::new()
    });
    let actor = test_new_actor(new_actor);

    assert_eq!(called_new_count.load(Ordering::Relaxed), 2);
    assert_eq!(actor.handle_call_count, 1);
}

#[test]
fn actor_reuse_factory() {
    let called_new_count = AtomicUsize::new(0);
    let new_actor = ActorReuseFactory::new(|| {
        called_new_count.fetch_add(1, Ordering::Relaxed);
        TestActor::new()
    }, |_| {}); // Don't do anything for reuse.
    let actor = test_new_actor(new_actor);

    assert_eq!(called_new_count.load(Ordering::Relaxed), 1);
    assert_eq!(actor.handle_call_count, 2);
}

/// Creates a new actor, calls it once and makes sure the return value is ok.
/// Then reuses the actor, calls it again, and returns it.
fn test_new_actor<N, M, A, F>(new_actor: N) -> A
    where N: NewActor<Message = M, Error = (), Actor = A>,
          M: Default,
          A: Actor<Message = M, Error = (), Future = F>,
          F: Future<Item = (), Error = ()>,
{
    let mut actor = new_actor.new();
    assert!(actor.handle(Default::default()).wait().is_ok());
    new_actor.reuse(&mut actor);
    assert!(actor.handle(Default::default()).wait().is_ok());
    actor
}
