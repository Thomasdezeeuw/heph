use std::task;
use std::time::{Duration, Instant};

use heph::actor::{self, actor_fn};
use heph::supervisor::StopSupervisor;
use heph_rt::setup::{TimerToken, Timers};
use heph_rt::spawn::ActorOptions;
use heph_rt::timer::Timer;
use heph_rt::{self as rt, Runtime, ThreadLocal};

// An example that show how to use a custom timers implementation.
//
// Run using:
// $ cargo run --example 9_runtime_setup
fn main() -> Result<(), rt::Error> {
    std_logger::Config::logfmt().init();

    let mut runtime = Runtime::setup()
        // Use our custom timers implementation.
        .with_timers(MyTimers::new)
        .build()?;

    runtime.run_on_workers(|mut rt| {
        let actor = actor_fn(greeter_actor);
        rt.spawn_local(StopSupervisor, actor, (), ActorOptions::default());
        Ok::<(), rt::Error>(())
    })?;
    runtime.start()
}

async fn greeter_actor(ctx: actor::Context<(), ThreadLocal>) {
    Timer::after(ctx.runtime_ref().clone(), Duration::from_millis(100)).await;
    println!("Hello, World!");
}

#[derive(Debug)]
struct MyTimers(Vec<(Instant, task::Waker)>);

impl MyTimers {
    fn new() -> MyTimers {
        MyTimers(Vec::new())
    }
}

impl Timers for MyTimers {
    fn next_deadline(&mut self) -> Option<Instant> {
        self.0.first().map(|(i, _)| *i)
    }

    fn expire_timers(&mut self, now: Instant) -> usize {
        let mut amount = 0;
        while let Some((deadline, _)) = self.0.first() {
            if *deadline >= now {
                // Not yet expired.
                break;
            }
            println!("Waking timer: {deadline:?}");
            let (_, waker) = self.0.remove(0);
            waker.wake();
            amount += 1;
        }
        amount
    }

    fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        println!("Adding timer: {deadline:?}");
        self.0.push((deadline, waker));
        self.0.sort_by(|(i1, _), (i2, _)| i1.cmp(i2));
        TimerToken::new(0) // Not used in this example.
    }

    fn remove(&mut self, deadline: Instant, _token: TimerToken) {
        println!("Not removing timer: {deadline:?}");
        // Not implemented for this example.
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}
