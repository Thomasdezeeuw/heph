use heph::rt::{self, ActorOptions, Runtime, RuntimeRef};
use heph::{actor, restart_supervisor};

fn main() -> Result<(), rt::Error> {
    heph::log::init();

    Runtime::new()?
        .with_setup(|mut runtime_ref: RuntimeRef| {
            let print_actor = print_actor as fn(_, _) -> _;
            let options = ActorOptions::default().mark_ready();
            let arg = "Hello world!".to_owned();
            let supervisor = PrintSupervisor::new(arg.clone());
            runtime_ref.spawn_local(supervisor, print_actor, arg, options);
            Ok(())
        })
        .start()
}

// Create a restart supervisor for the [`print_actor`].
restart_supervisor!(PrintSupervisor, "print actor", String);

/// A very bad printing error.
async fn print_actor(_ctx: actor::Context<()>, _msg: String) -> Result<(), String> {
    Err("can't print!".to_owned())
}
