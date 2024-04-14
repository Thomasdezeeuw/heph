use heph::actor::actor_fn;
use heph::supervisor::NoSupervisor;
use heph::sync::{self, SyncActorRunner};

fn main() {
    // `SyncActorRunner` can be used to run synchronous actors. This is similar
    // to `ActorFuture`, but runs sync actors (instead of async actors).
    let (actor, actor_ref) = SyncActorRunner::new(NoSupervisor, actor_fn(sync_actor));

    // Just like with any actor reference we can send the actor a message.
    actor_ref.try_send("Hello world".to_string()).unwrap();
    // We need to drop the reference here to ensure the actor stops.
    drop(actor_ref);

    // Unlike asynchronous actor we don't need a `Future` runtime to run
    // synchronous actors, they can be run like a normal function.
    let args = "Bye";
    actor.run(args);
}

fn sync_actor(mut ctx: sync::Context<String>, exit_msg: &'static str) {
    while let Ok(msg) = ctx.receive_next() {
        println!("Got a message: {msg}");
    }
    println!("{exit_msg}");
}
