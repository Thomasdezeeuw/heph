//! Tests for spawning [`Future`]s.

use std::future::Future;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::{ActorOptions, FutureOptions};
use heph_rt::test::poll_future;
use heph_rt::{Runtime, ThreadSafe};

use crate::util::{expect_pending, expect_ready};

struct TestFuture {
    wakes: usize,
}

const fn test_future() -> TestFuture {
    TestFuture { wakes: 0 }
}

impl Future for TestFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        match self.wakes {
            0 => {
                ctx.waker().wake_by_ref();
                self.wakes += 1;
                Poll::Pending
            }
            1 => {
                ctx.waker().clone().wake();
                self.wakes += 1;
                Poll::Pending
            }
            _ => Poll::Ready(()),
        }
    }
}

#[test]
fn test_poll_future() {
    let mut future = Box::pin(test_future());

    expect_pending(poll_future(future.as_mut()));
    expect_pending(poll_future(future.as_mut()));
    expect_ready(poll_future(future.as_mut()), ());
}

#[test]
fn thread_local_waking() {
    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers::<_, !>(|mut runtime_ref| {
            runtime_ref.spawn_local_future(test_future(), FutureOptions::default());
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}

#[test]
fn thread_safe_waking() {
    async fn spawn_actor(mut ctx: actor::Context<!, ThreadSafe>) {
        // Spawn on `ThreadSafe`.
        ctx.runtime()
            .spawn_future(test_future(), FutureOptions::default());
    }

    let mut runtime = Runtime::new().unwrap();
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(spawn_actor),
        (),
        ActorOptions::default(),
    );
    // Spawn on `Runtime`.
    runtime.spawn_future(test_future(), FutureOptions::default());
    runtime
        .run_on_workers::<_, !>(|mut runtime_ref| {
            // Spawn on `Runtime_ref`.
            runtime_ref.spawn_future(test_future(), FutureOptions::default());
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}
