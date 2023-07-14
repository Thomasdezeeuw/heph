//! Tests for the [`Actor`] trait.

use heph::actor::{self, actor_fn, NewActor};

#[test]
fn future_output_result() {
    // Actor is implemented for `Future<Output = Result<(), E>>`.
    async fn actor(_: actor::Context<(), ()>) -> Result<(), ()> {
        Ok(())
    }
    is_new_actor(actor_fn(actor));
}

#[test]
fn future_output_tuple() {
    // Actor is implemented for `Future<Output = ()>`.
    async fn actor(_: actor::Context<(), ()>) {}
    is_new_actor(actor_fn(actor));
}

fn is_new_actor<NA: NewActor>(_: NA) {}
