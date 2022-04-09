use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};
use std::thread;
use std::time::{Duration, Instant};

use heph_rt::actor::{self, Bound};
use heph_rt::rt::{self, Runtime, RuntimeRef, ThreadLocal};
use heph_rt::spawn::ActorOptions;
use heph_rt::supervisor::NoSupervisor;
use heph_rt::test::{init_local_actor, poll_actor, poll_future, poll_next};
use heph_rt::timer::{Deadline, DeadlinePassed, Interval, Timer};
use heph_rt::util::next;
use heph_rt::ActorRef;

use crate::util::{count_polls, expect_pending};

const SMALL_TIMEOUT: Duration = Duration::from_millis(50);
const TIMEOUT: Duration = Duration::from_millis(100);

#[test]
fn deadline_passed_into_io_error() {
    let err: io::Error = DeadlinePassed.into();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn timer() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let mut timer = Timer::after(&mut ctx, TIMEOUT);
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(!timer.has_passed());

        let _ = (&mut timer).await;
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(timer.has_passed());
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    thread::sleep(TIMEOUT);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
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
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let future = AlwaysPending;
        let mut deadline = Timer::after(&mut ctx, TIMEOUT).wrap(future);
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(!deadline.has_passed());

        let res: Result<(), DeadlinePassed> = (&mut deadline).await;
        assert_eq!(res, Err(DeadlinePassed));
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(deadline.has_passed());
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    thread::sleep(TIMEOUT);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn deadline() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let future = AlwaysPending;
        let mut deadline = Deadline::after(&mut ctx, TIMEOUT, future.clone());
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
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    thread::sleep(TIMEOUT);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn interval() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let start = Instant::now();
        let mut interval = Interval::every(&mut ctx, TIMEOUT);
        assert!(interval.next_deadline() >= start + TIMEOUT);
        let _ = next(&mut interval).await;
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    thread::sleep(TIMEOUT);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn triggered_timers_run_actors() {
    async fn timer_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let timer = Timer::after(&mut ctx, TIMEOUT);
        let _ = timer.await;
    }

    async fn deadline_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let future = AlwaysPending;
        let deadline = Deadline::after(&mut ctx, TIMEOUT, future);
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
    }

    async fn interval_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone + Unpin,
    {
        let mut interval = Interval::every(&mut ctx, TIMEOUT);
        let _ = next(&mut interval).await;
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor as fn(_) -> _,
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
        timer_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        deadline_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        interval_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    runtime.start().unwrap();
}

#[test]
fn timers_dont_trigger_after_drop() {
    async fn timer_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        // Setup an initial timer.
        let mut timer = Timer::after(&mut ctx, SMALL_TIMEOUT);
        expect_pending(poll_future(Pin::new(&mut timer)));
        // Dropping it should remove the timer.
        drop(timer);

        let timer = Timer::after(&mut ctx, TIMEOUT);
        let (_, poll_count) = count_polls(timer).await;
        // Should only be polled twice, the first time the deadline
        // hasn't passed, but the second time its called it should.
        assert_eq!(poll_count, 2);
    }

    async fn deadline_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone,
    {
        let mut deadline = Deadline::after(&mut ctx, SMALL_TIMEOUT, AlwaysPending);
        expect_pending(poll_future(Pin::new(&mut deadline)));
        drop(deadline);

        let deadline = Deadline::after(&mut ctx, TIMEOUT, AlwaysPending);
        let (_, poll_count) = count_polls(deadline).await;
        assert_eq!(poll_count, 2);
    }

    async fn interval_actor<RT>(mut ctx: actor::Context<!, RT>)
    where
        RT: rt::Access + Clone + Unpin,
    {
        let mut interval = Interval::every(&mut ctx, SMALL_TIMEOUT);
        expect_pending(poll_next(Pin::new(&mut interval)));
        drop(interval);

        let interval = Interval::every(&mut ctx, TIMEOUT);
        let (_, poll_count) = next(count_polls(interval)).await.unwrap();
        assert_eq!(poll_count, 2);
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor as fn(_) -> _,
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
        timer_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        deadline_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        interval_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}

#[test]
fn timers_actor_bound() {
    async fn timer_actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Timer<RT>>)
    where
        RT: rt::Access + Clone,
    {
        let timer = Timer::after(&mut ctx, TIMEOUT);
        actor_ref.send(timer).await.unwrap();
    }

    async fn timer_actor2<RT>(mut ctx: actor::Context<Timer<RT>, RT>)
    where
        RT: rt::Access + Clone,
    {
        let mut timer = ctx.receive_next().await.unwrap();
        timer.bind_to(&mut ctx).unwrap();
        let _ = timer.await;
    }

    async fn deadline_actor1<RT>(
        mut ctx: actor::Context<!, RT>,
        actor_ref: ActorRef<Deadline<AlwaysPending, RT>>,
    ) where
        RT: rt::Access + Clone,
    {
        let future = AlwaysPending;
        let deadline = Deadline::after(&mut ctx, TIMEOUT, future);
        actor_ref.send(deadline).await.unwrap();
    }

    async fn deadline_actor2<RT>(mut ctx: actor::Context<Deadline<AlwaysPending, RT>, RT>)
    where
        RT: rt::Access,
    {
        let mut deadline = ctx.receive_next().await.unwrap();
        deadline.bind_to(&mut ctx).unwrap();
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
    }

    async fn interval_actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Interval<RT>>)
    where
        RT: rt::Access + Clone,
    {
        let interval = Interval::every(&mut ctx, TIMEOUT);
        actor_ref.send(interval).await.unwrap();
    }

    async fn interval_actor2<RT>(mut ctx: actor::Context<Interval<RT>, RT>)
    where
        RT: rt::Access + Unpin,
    {
        let mut interval = ctx.receive_next().await.unwrap();
        interval.bind_to(&mut ctx).unwrap();
        let _ = next(&mut interval).await;
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let actor_ref = runtime.spawn(
        NoSupervisor,
        timer_actor2 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        timer_actor1 as fn(_, _) -> _,
        actor_ref,
        ActorOptions::default(),
    );
    let actor_ref = runtime.spawn(
        NoSupervisor,
        deadline_actor2 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        deadline_actor1 as fn(_, _) -> _,
        actor_ref,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}

#[test]
fn timers_dont_trigger_after_actor_bound() {
    async fn timer_actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Timer<RT>>)
    where
        RT: rt::Access + Clone,
    {
        // Setup an initial timer.
        let mut timer = Timer::after(&mut ctx, SMALL_TIMEOUT);
        expect_pending(poll_future(Pin::new(&mut timer)));
        // Letting another bind it should remove the timer.
        actor_ref.send(timer).await.unwrap();

        let timer = Timer::after(&mut ctx, TIMEOUT);
        let (_, poll_count) = count_polls(timer).await;
        // Should only be polled twice, the first time the deadline
        // hasn't passed, but the second time its called it should.
        assert_eq!(poll_count, 2);
    }

    async fn timer_actor2<RT>(mut ctx: actor::Context<Timer<RT>, RT>)
    where
        RT: rt::Access + Clone,
    {
        let mut timer = ctx.receive_next().await.unwrap();
        timer.bind_to(&mut ctx).unwrap();
        let _ = timer.await;
    }

    async fn deadline_actor1<RT>(
        mut ctx: actor::Context<!, RT>,
        actor_ref: ActorRef<Deadline<AlwaysPending, RT>>,
    ) where
        RT: rt::Access + Clone,
    {
        let mut deadline = Deadline::after(&mut ctx, SMALL_TIMEOUT, AlwaysPending);
        expect_pending(poll_future(Pin::new(&mut deadline)));
        actor_ref.send(deadline).await.unwrap();

        let deadline = Deadline::after(&mut ctx, TIMEOUT, AlwaysPending);
        let (_, poll_count) = count_polls(deadline).await;
        assert_eq!(poll_count, 2);
    }

    async fn deadline_actor2<RT>(mut ctx: actor::Context<Deadline<AlwaysPending, RT>, RT>)
    where
        RT: rt::Access,
    {
        let mut deadline = ctx.receive_next().await.unwrap();
        deadline.bind_to(&mut ctx).unwrap();
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
    }

    async fn interval_actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Interval<RT>>)
    where
        RT: rt::Access + Clone,
    {
        let mut interval = Interval::every(&mut ctx, SMALL_TIMEOUT);
        expect_pending(poll_next(Pin::new(&mut interval)));
        actor_ref.send(interval).await.unwrap();

        let interval = Interval::every(&mut ctx, TIMEOUT);
        let (_, poll_count) = next(count_polls(interval)).await.unwrap();
        assert_eq!(poll_count, 2);
    }

    async fn interval_actor2<RT>(mut ctx: actor::Context<Interval<RT>, RT>)
    where
        RT: rt::Access + Unpin,
    {
        let mut interval = ctx.receive_next().await.unwrap();
        interval.bind_to(&mut ctx).unwrap();
        let _ = next(&mut interval).await;
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            timer_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            deadline_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            interval_actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let actor_ref = runtime.spawn(
        NoSupervisor,
        timer_actor2 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        timer_actor1 as fn(_, _) -> _,
        actor_ref,
        ActorOptions::default(),
    );
    let actor_ref = runtime.spawn(
        NoSupervisor,
        deadline_actor2 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        deadline_actor1 as fn(_, _) -> _,
        actor_ref,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}
