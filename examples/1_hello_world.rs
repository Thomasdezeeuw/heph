#![feature(never_type)]

use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};

fn main() -> Result<(), rt::Error> {
    // We create a new runtime. Add a setup function, which adds our greeter
    // actor. And finally we start it.
    Runtime::new()?.with_setup(add_greeter_actor).start()
}

/// The is the setup function used in the runtime.
fn add_greeter_actor(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    // spawn our `greeter_actor` onto the runtime.
    // All actors need supervision, however our actor doesn't return an error
    // (it uses `!`, the never type, as error), because of this we'll use the
    // `NoSupervisor`, which is a supervisor that does nothing and can't be
    // called.
    // Along with the supervisor we'll also supply the argument to start the
    // actor, in our case this is `()` since our actor doesn't accept any
    // arguments.
    // We'll use the default actor options here, other examples expand on the
    // options available.
    let actor = greeter_actor as fn(_) -> _;
    let actor_ref = runtime_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());

    // By default actors don't do anything when spawned. We need to wake them,
    // for example by sending them a message. If we didn't send this message the
    // runtime would run forever, without ever making progress (try this by
    // commenting out the send below!). So we'll send our actor a message via an
    // actor reference, which is a reference to the actor inside the runtime.
    actor_ref.send("World").unwrap();

    Ok(())
}

/// Our greeter actor.
///
/// We'll receive a single message and print it.
async fn greeter_actor(mut ctx: actor::Context<&'static str>) -> Result<(), !> {
    // All actors have an actor context, which give the actor access to, among
    // other things, its inbox from which we can receive a message.
    let name = ctx.receive_next().await;
    println!("Hello {}", name);
    Ok(())
}
