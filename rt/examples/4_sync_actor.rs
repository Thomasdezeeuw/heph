use heph::actor::actor_fn;
use heph::supervisor::NoSupervisor;
use heph::sync;
use heph_rt::spawn::SyncActorOptions;
use heph_rt::{self as rt, Runtime};

fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // Spawning synchronous actor works slightly differently the spawning
    // regular (asynchronous) actors. Mainly, synchronous actors need to be
    // spawned before the runtime is started.
    let mut runtime = Runtime::new()?;

    // Spawn a new synchronous actor, returning an actor reference to it.
    let actor = actor_fn(actor);
    // Options used to spawn the synchronous actor.
    // Here we'll set the name of
    // the thread that runs the actor.
    let options = SyncActorOptions::default().with_thread_name("My actor".to_owned());
    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor, "Bye", options)?;

    // Just like with any actor reference we can send the actor a message.
    actor_ref.try_send("Hello world".to_string()).unwrap();
    // We need to drop the reference here to ensure the actor stops.
    // The actor contains a `while` loop receiving messages (see the `actor`
    // function), that only stops iterating once all actor reference to it are
    // dropped or waits otherwise. If didn't manually dropped the reference here
    // it would be dropped only after `runtime.start` returned below, when it
    // goes out of scope. However that will never happen as the `actor` will
    // wait until its dropped.
    drop(actor_ref);

    // And now we start the runtime.
    runtime.start()
}

fn actor<RT>(mut ctx: sync::Context<String, RT>, exit_msg: &'static str) {
    while let Ok(msg) = ctx.receive_next() {
        println!("Got a message: {msg}");
    }
    println!("{exit_msg}");
}
