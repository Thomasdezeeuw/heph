#![feature(never_type)]

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::ActorOptions;
use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};

fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // We create a new runtime. Add a setup function, which adds our greeter
    // actor. And finally we start it.
    let mut runtime = Runtime::setup().build()?;
    runtime.run_on_workers(add_greeter_actor)?;
    runtime.start()
}

/// The is the setup function used in the runtime.
fn add_greeter_actor(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    // Spawn our `greeter_actor` onto the runtime.
    // All actors need supervision, however our actor doesn't return an error
    // (it uses `!`, the never type, as error), because of this we'll use the
    // `NoSupervisor`, which is a supervisor that does nothing and can't be
    // called.
    // Along with the supervisor we'll also supply the argument to start the
    // actor, in our case this is `()` since our actor doesn't accept any
    // arguments.
    // We'll use the default actor options here, other examples expand on the
    // options available.
    let actor = actor_fn(greeter_actor);
    let actor_ref = runtime_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());

    // Now we can send our actor a message using its `actor_ref`.
    actor_ref.try_send("World").unwrap();
    Ok(())
}

/// Our greeter actor.
///
/// We'll receive a single message and print it.
async fn greeter_actor(mut ctx: actor::Context<&'static str, ThreadLocal>) {
    // All actors have an actor context, which give the actor access to, among
    // other things, its inbox from which it can receive a message.
    while let Ok(name) = ctx.receive_next().await {
        println!("Hello {name}");
    }
}
