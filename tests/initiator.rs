//! Tests for the `initiator` module.

use actor::initiator::{Initiator, NoInitiator};
use actor::system::{ActorSystem, ActorSystemBuilder};

fn default_actor_system() -> ActorSystem {
    ActorSystemBuilder::default().build()
        .expect("unable to build actor system")
}

#[test]
fn no_initiator_does_nothing() {
    let actor_system = default_actor_system();
    let mut system_ref = actor_system.create_ref();
    let mut initiator = NoInitiator;
    assert!(initiator.poll(&mut system_ref).is_ok());
}
