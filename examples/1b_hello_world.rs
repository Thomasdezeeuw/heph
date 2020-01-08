#![feature(never_type)]

use heph::supervisor::NoSupervisor;
use heph::{actor, ActorOptions, Runtime, RuntimeError, RuntimeRef};

// The creation and running of the actor system is the same as in example 1.
fn main() -> Result<(), RuntimeError> {
    Runtime::new().with_setup(add_greeter_actor).start()
}

/// The is the setup function used in the actor system.
fn add_greeter_actor(mut system_ref: RuntimeRef) -> Result<(), !> {
    // As shown in example 1 actors don't do anything went they are not awoken.
    // In example 1 we send the actor a message to wake it, this example will
    // use the `schedule` actor option.
    // The `schedule` actor option will wake (schedule) the actor when it is
    // added to the actor system for the first time. This is useful for actors
    // that don't have any (initial) external wakers, for example in the case of
    // our `greeter_actor` below.
    let options = ActorOptions::default().schedule();
    let actor = greeter_actor as fn(_) -> _;
    system_ref.spawn(NoSupervisor, actor, (), options);

    Ok(())
}

/// Our greeter actor.
///
/// Note: this needs the `schedule` options when adding it to the actor system.
async fn greeter_actor(_: actor::Context<!>) -> Result<(), !> {
    println!("Hello World");
    Ok(())
}
