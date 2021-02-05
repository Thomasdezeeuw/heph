#![cfg(feature = "test")]

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::stream::StreamExt;

use heph::actor::{self, Bound};
use heph::supervisor::NoSupervisor;
use heph::test::{init_local_actor, poll_actor};
use heph::timer::{Deadline, DeadlinePassed, Interval, Timer};
use heph::{rt, ActorOptions, ActorRef, Runtime, RuntimeRef};

const TIMEOUT: Duration = Duration::from_millis(100);

#[test]
fn deadline_passed_into_io_error() {
    let err: io::Error = DeadlinePassed.into();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn timer() {
    async fn actor(mut ctx: actor::Context<!>) -> Result<(), !> {
        let start = Instant::now();
        let mut timer = Timer::timeout(&mut ctx, TIMEOUT);
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(!timer.has_passed());

        let _ = (&mut timer).await;
        assert!(timer.deadline() >= start + TIMEOUT);
        assert!(timer.has_passed());
        Ok(())
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    thread::sleep(TIMEOUT);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Pending(u8);

impl Future for Pending {
    type Output = Result<(), DeadlinePassed>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

#[test]
fn timer_wrap() {
    async fn actor(mut ctx: actor::Context<!>) -> Result<(), !> {
        let start = Instant::now();
        let future = Pending(123);
        let mut deadline = Timer::timeout(&mut ctx, TIMEOUT).wrap(future);
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(!deadline.has_passed());

        let res: Result<(), DeadlinePassed> = (&mut deadline).await;
        assert_eq!(res, Err(DeadlinePassed));
        assert!(deadline.deadline() >= start + TIMEOUT);
        assert!(deadline.has_passed());
        Ok(())
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
    async fn actor(mut ctx: actor::Context<!>) -> Result<(), !> {
        let start = Instant::now();
        let future = Pending(123);
        let mut deadline = Deadline::timeout(&mut ctx, TIMEOUT, future.clone());
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
        Ok(())
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
    async fn actor(mut ctx: actor::Context<!>) -> Result<(), !> {
        let start = Instant::now();
        let mut interval = Interval::new(&mut ctx, TIMEOUT);
        assert!(interval.next_deadline() >= start + TIMEOUT);
        let _ = interval.next().await;
        Ok(())
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
    async fn timer_actor<K>(mut ctx: actor::Context<!, K>) -> Result<(), !>
    where
        actor::Context<!, K>: rt::Access,
    {
        let timer = Timer::timeout(&mut ctx, TIMEOUT);
        let _ = timer.await;
        Ok(())
    }

    async fn deadline_actor<K>(mut ctx: actor::Context<!, K>) -> Result<(), !>
    where
        actor::Context<!, K>: rt::Access,
    {
        let future = Pending(123);
        let deadline = Deadline::timeout(&mut ctx, TIMEOUT, future);
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
        Ok(())
    }

    async fn interval_actor(mut ctx: actor::Context<!>) -> Result<(), !> {
        let mut interval = Interval::new(&mut ctx, TIMEOUT);
        let _ = interval.next().await;
        Ok(())
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

    let mut runtime = Runtime::new().unwrap().with_setup(setup);

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

    runtime.start().unwrap();
}

#[test]
fn timers_actor_bound() {
    async fn timer_actor1<K>(
        mut ctx: actor::Context<!, K>,
        actor_ref: ActorRef<Timer>,
    ) -> Result<(), !>
    where
        actor::Context<!, K>: rt::Access,
    {
        let timer = Timer::timeout(&mut ctx, TIMEOUT);
        actor_ref.send(timer).await.unwrap();
        Ok(())
    }

    async fn timer_actor2<K>(mut ctx: actor::Context<Timer, K>) -> Result<(), !>
    where
        actor::Context<Timer, K>: rt::Access,
    {
        let mut timer = ctx.receive_next().await.unwrap();
        timer.bind_to(&mut ctx).unwrap();
        let _ = timer.await;
        Ok(())
    }

    async fn deadline_actor1<K>(
        mut ctx: actor::Context<!, K>,
        actor_ref: ActorRef<Deadline<Pending>>,
    ) -> Result<(), !>
    where
        actor::Context<!, K>: rt::Access,
    {
        let future = Pending(123);
        let deadline = Deadline::timeout(&mut ctx, TIMEOUT, future);
        actor_ref.send(deadline).await.unwrap();
        Ok(())
    }

    async fn deadline_actor2<K>(mut ctx: actor::Context<Deadline<Pending>, K>) -> Result<(), !>
    where
        actor::Context<Deadline<Pending>, K>: rt::Access,
    {
        let mut deadline = ctx.receive_next().await.unwrap();
        deadline.bind_to(&mut ctx).unwrap();
        let res: Result<(), DeadlinePassed> = deadline.await;
        assert_eq!(res, Err(DeadlinePassed));
        Ok(())
    }

    async fn interval_actor1(
        mut ctx: actor::Context<!>,
        actor_ref: ActorRef<Interval>,
    ) -> Result<(), !> {
        let interval = Interval::new(&mut ctx, TIMEOUT);
        actor_ref.send(interval).await.unwrap();
        Ok(())
    }

    async fn interval_actor2(mut ctx: actor::Context<Interval>) -> Result<(), !> {
        let mut interval = ctx.receive_next().await.unwrap();
        interval.bind_to(&mut ctx).unwrap();
        let _ = interval.next().await;
        Ok(())
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

    let mut runtime = Runtime::new().unwrap().with_setup(setup);

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
