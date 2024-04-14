use heph::actor::{self, actor_fn};
use heph::future::ActorFuture;
use heph::supervisor::NoSupervisor;

mod runtime; // Replace this with your favourite `Future` runtime.

fn main() {
    // First we wrap our actor in an `ActorFuture` so that it implements the
    // `Future` trait, which also gives us an `ActorRef` so we can send it
    // messages.
    //
    // All actors need supervision, however our actor doesn't return an error
    // (it uses `!`, the never type, as error), because of this we'll use the
    // `NoSupervisor`, which is a supervisor that does nothing and can't be
    // called. For more information on actor supervision see the `supervisor`
    // module.
    let supervisor = NoSupervisor;
    // An unfortunate implementation detail requires us to convert our
    // asynchronous function to an `NewActor` implementation (trait that defines
    // how actors are (re)started).
    // The easiest way to do this is to use the `actor_fn` helper function. See
    // the documentation of `actor_fn` for more details on why this is required.
    let actor = actor_fn(greeter_actor);
    // We'll also supply the argument to start the actor, in our case this is
    // `()` since our actor doesn't accept any arguments.
    let args = ();

    // Now that we have all the arguments prepare we can create out `Future`
    // that represents our actor.
    let (future, actor_ref) = ActorFuture::new(supervisor, actor, args).unwrap();

    // Now we can send our actor a message using its `actor_ref`.
    actor_ref.try_send("World").unwrap();

    // We need to drop the `actor_ref` to ensure the actor stops.
    drop(actor_ref);

    // We run our `future` on our runtime.
    runtime::block_on(future);
}

/// Our greeter actor.
///
/// We'll receive a single message and print it.
async fn greeter_actor(mut ctx: actor::Context<&'static str>) {
    // All actors have an actor context, which give the actor access to, among
    // other things, its inbox from which it can receive a message.
    while let Ok(name) = ctx.receive_next().await {
        println!("Hello {name}");
    }
}
