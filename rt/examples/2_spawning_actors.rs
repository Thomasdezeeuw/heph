#![feature(never_type)]

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph::sync;
use heph_rt::access::Sync;
use heph_rt::spawn::{ActorOptions, SyncActorOptions};
use heph_rt::{self as rt, Runtime, RuntimeRef};

// This example shows how to:
//  1. Setup and run a Runtime,
//  2. Spawn a synchronous actor,
//  3. Spawn a thread-safe actor, and
//  4. Spawn a thread-local actor.
//
// When this runs this should print four messages:
//  * One from the synchronous actor.
//  * One from the asynchronous thread-safe actor.
//  * One from the asynchronous thread-local actor on each worker thread, thus
//    two in total.
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // Build a new Runtime.
    let mut runtime = Runtime::setup()
        // We can set the application name.
        .with_name("my_app".into())
        // Set the number of worker threads to two.
        .num_threads(2)
        // Finally we build the runtime.
        .build()?;

    // Now the runtime is build we can start spawning our actors and futures.
    // We'll start with spawning a synchronous actor. For more information on
    // spawning actors see the spawn module.
    //
    // All actors need supervision and thus a supervisor. However our actor
    // doesn't return an error (it uses "!", the never type, as error). Because
    // of this we'll use the NoSupervisor, which is a supervisor that does
    // nothing and can't be called (and can only be used with the never type as
    // error).
    let supervisor = NoSupervisor;
    // Along with the supervisor we'll need the implementation to start a new
    // actor, this is defined in the NewActor trait.
    //
    // Due to type system restrictions asynchronous functions don't implement
    // the required NewActor trait directly. The simplest solution is to wrap it
    // using actor_fn to implement the trait.
    let sync_actor = actor_fn(sync_actor);
    // We'll also supply the argument(s) to start the actor, in our case this is
    // "()" since our actor doesn't accept any arguments.
    let args = ();
    // The last argument are options we'll use to spawn the actor. Here we set
    // name of the thread that will run our synchronous actor.
    let options = SyncActorOptions::default().with_thread_name("sync actor".into());
    // With all the arguments prepared we can spawn the actor. We get back an
    // ActorRef which we can use to send the actor messages and check if it's
    // still running.
    let actor_ref = runtime.spawn_sync_actor(supervisor, sync_actor, args, options)?;
    // Here we use the actor reference to send a message to the spawned actor.
    actor_ref.try_send("Alice").unwrap();

    // Next we spawn a thread-safe actor.
    //
    // The arguments are mostly the same as for the synchronous actor above, but
    // use slightly different trait bounds and types.
    let actor_ref = runtime.spawn(
        NoSupervisor,
        actor_fn(actor),
        "thread-safe",
        ActorOptions::default(),
    );
    // Send a thread-safe actor a message is exactly the same as for synchronous
    // actor.
    actor_ref.try_send("Bob").unwrap();

    // Finally we can spawn thread-local actors. This a little more difficult as
    // we can only do that on the thread on which the actor will run. So to do
    // so we can run a function on each worker thread using
    // Runtime::run_on_workers, which simply runs a given function on all worker
    // threads.
    runtime.run_on_workers(|mut rt: RuntimeRef| -> Result<(), rt::Error> {
        // Spawning a thread-local actor is the same as spawning a thread-safe
        // actor, but doesn't require the Send or Sync trait bounds.
        let actor_ref = rt.spawn_local(
            NoSupervisor,
            actor_fn(actor),
            "thread-local",
            ActorOptions::default(),
        );
        // Sending a thread-local actor a message is the same as for synchronous
        // and thread-safe actors.
        actor_ref.try_send("Charlie").unwrap();
        Ok(())
    })?;

    // And once the setup is complete we can start the runtime.
    // This will run all the actors we spawned.
    runtime.start()
}

/// Our synchronous actor.
fn sync_actor(mut ctx: sync::Context<&'static str, Sync>) {
    if let Ok(name) = ctx.receive_next() {
        println!("Hello {name} from sync actor");
    }
}

/// Our asynchronous actor that can be run as a thread-local or thread-safe actor.
async fn actor<RT>(mut ctx: actor::Context<&'static str, RT>, actor_kind: &'static str) {
    if let Ok(name) = ctx.receive_next().await {
        println!("Hello {name} from {actor_kind} actor");
    }
}
