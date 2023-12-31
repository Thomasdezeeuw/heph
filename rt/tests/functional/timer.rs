use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{block_on_local_actor, poll_future, poll_next};
use heph_rt::timer::{Deadline, DeadlinePassed, Interval, Timer};
use heph_rt::util::next;
use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal, ThreadSafe};

use crate::util::{assert_size, count_polls, expect_pending};

const SMALL_TIMEOUT: Duration = Duration::from_millis(50);
const TIMEOUT: Duration = Duration::from_millis(100);

#[test]
fn size() {
    assert_size::<Timer<ThreadLocal>>(40);
    assert_size::<Timer<ThreadSafe>>(40);
    assert_size::<Deadline<(), ThreadLocal>>(40);
    assert_size::<Deadline<(), ThreadSafe>>(40);
    assert_size::<Interval<ThreadLocal>>(56);
    assert_size::<Interval<ThreadSafe>>(56);
    assert_size::<DeadlinePassed>(0);
}

#[test]
fn deadline_passed_into_io_error() {
    let err: io::Error = DeadlinePassed.into();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn timer() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let mut timer = Timer::after(ctx.runtime_ref().clone(), TIMEOUT);
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(!timer.has_passed());

        let _ = (&mut timer).await;
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(timer.has_passed());
        assert!(start.elapsed() >= TIMEOUT);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn timer_ref_rt() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        // Instead of cloning `ThreadLocal`, we can use a mutable reference to
        // it.
        let mut timer = Timer::after(ctx.runtime(), TIMEOUT);
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(!timer.has_passed());

        let _ = (&mut timer).await;
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(timer.has_passed());
        assert!(start.elapsed() >= TIMEOUT);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AlwaysPending;

impl Future for AlwaysPending {
    type Output = Result<(), DeadlinePassed>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

#[test]
fn timer_wrap() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let future = AlwaysPending;
        let mut deadline = Timer::after(ctx.runtime_ref().clone(), TIMEOUT).wrap(future);
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(!deadline.has_passed());

        let res: Result<(), DeadlinePassed> = (&mut deadline).await;
        assert_eq!(res, Err(DeadlinePassed));
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(deadline.has_passed());
        assert!(start.elapsed() >= TIMEOUT);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn deadline() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let future = AlwaysPending;
        let mut deadline = Deadline::after(ctx.runtime_ref().clone(), TIMEOUT, future.clone());
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(!deadline.has_passed());
        assert_eq!(*deadline.get_ref(), future);
        assert_eq!(*deadline.get_mut(), future);

        let res: Result<(), DeadlinePassed> = (&mut deadline).await;
        assert_eq!(res, Err(DeadlinePassed));
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(deadline.has_passed());
        assert_eq!(*deadline.get_ref(), future);
        assert_eq!(*deadline.get_mut(), future);
        assert_eq!(deadline.into_inner(), future);
        assert!(start.elapsed() >= TIMEOUT);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn interval() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let mut interval = Interval::every(ctx.runtime_ref().clone(), TIMEOUT);
        assert!(interval.next_deadline() >= start + TIMEOUT);
        let _ = next(&mut interval).await;
        assert!(start.elapsed() >= TIMEOUT);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn triggered_timers_run_actors() {
    async fn timer_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let timer = Timer::after(ctx.runtime_ref().clone(), TIMEOUT);
        let _ = timer.await;
    }

    async fn deadline_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let future = AlwaysPending;
        let deadline = Deadline::after(ctx.runtime_ref().clone(), TIMEOUT, future);
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
    }

    async fn interval_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone + Unpin,
    {
        let mut interval = Interval::every(ctx.runtime_ref().clone(), TIMEOUT);
        let _ = next(&mut interval).await;
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(timer_actor),
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(deadline_actor),
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(interval_actor),
            (),
            ActorOptions::default(),
        );
        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(timer_actor),
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(deadline_actor),
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(interval_actor),
        (),
        ActorOptions::default(),
    );
    runtime.start().unwrap();
}

#[test]
fn timers_dont_trigger_after_drop() {
    async fn timer_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        // Setup an initial timer.
        let mut timer = Timer::after(ctx.runtime_ref().clone(), SMALL_TIMEOUT);
        expect_pending(poll_future(Pin::new(&mut timer)));
        // Dropping it should remove the timer.
        drop(timer);

        let timer = Timer::after(ctx.runtime_ref().clone(), TIMEOUT);
        let (_, poll_count) = count_polls(timer).await;
        // Should only be polled twice, the first time the deadline
        // hasn't passed, but the second time its called it should.
        assert_eq!(poll_count, 2);
    }

    async fn deadline_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let mut deadline = Deadline::after(ctx.runtime_ref().clone(), SMALL_TIMEOUT, AlwaysPending);
        expect_pending(poll_future(Pin::new(&mut deadline)));
        drop(deadline);

        let deadline = Deadline::after(ctx.runtime_ref().clone(), TIMEOUT, AlwaysPending);
        let (_, poll_count) = count_polls(deadline).await;
        assert_eq!(poll_count, 2);
    }

    async fn interval_actor<RT>(ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone + Unpin,
    {
        let mut interval = Interval::every(ctx.runtime_ref().clone(), SMALL_TIMEOUT);
        expect_pending(poll_next(Pin::new(&mut interval)));
        drop(interval);

        let interval = Interval::every(ctx.runtime_ref().clone(), TIMEOUT);
        let (_, poll_count) = next(count_polls(interval)).await.unwrap();
        assert_eq!(poll_count, 2);
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(timer_actor),
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(deadline_actor),
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor_fn(interval_actor),
            (),
            ActorOptions::default(),
        );
        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(timer_actor),
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(deadline_actor),
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(interval_actor),
        (),
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}
