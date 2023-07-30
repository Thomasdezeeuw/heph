use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::actor::{actor_fn, RecvError};
use heph::supervisor::{NoSupervisor, SupervisorStrategy};
use heph::sync;
use heph_rt::spawn::SyncActorOptions;
use heph_rt::test::spawn_sync_actor;

#[derive(Clone, Debug)]
struct BlockFuture {
    data: Arc<Mutex<(bool, Option<task::Waker>)>>,
}

impl BlockFuture {
    fn new() -> BlockFuture {
        BlockFuture {
            data: Arc::new(Mutex::new((false, None))),
        }
    }

    fn unblock(&self) {
        let mut data = self.data.lock().unwrap();
        data.0 = true;
        data.1.take().unwrap().wake();
    }

    fn wake(&self) {
        self.data.lock().unwrap().1.take().unwrap().wake_by_ref();
    }

    fn has_waker(&self) -> bool {
        self.data.lock().unwrap().1.is_some()
    }
}

impl Future for BlockFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut data = self.data.lock().unwrap();
        if data.0 {
            Poll::Ready(())
        } else {
            data.1 = Some(ctx.waker().clone());
            Poll::Pending
        }
    }
}

fn block_on_actor<RT, Fut>(mut ctx: sync::Context<String, RT>, fut: Fut)
where
    Fut: Future,
{
    let _ = ctx.block_on(fut);
}

#[test]
fn block_on() {
    let future = BlockFuture::new();

    let (handle, _) = spawn_sync_actor(
        NoSupervisor,
        actor_fn(block_on_actor),
        future.clone(),
        SyncActorOptions::default(),
    )
    .unwrap();

    while !future.has_waker() {
        sleep(Duration::from_millis(10));
    }

    future.unblock();
    handle.join().unwrap();
}

#[test]
fn block_on_spurious_wake_up() {
    let future = BlockFuture::new();

    let (handle, _) = spawn_sync_actor(
        NoSupervisor,
        actor_fn(block_on_actor),
        future.clone(),
        SyncActorOptions::default(),
    )
    .unwrap();

    // Wait until the future is polled a first time.
    while !future.has_waker() {
        sleep(Duration::from_millis(10));
    }
    // Wake up the sync actor, but don't yet let it continue.
    future.wake();

    // Wait until the sync actor is run again.
    while !future.has_waker() {
        sleep(Duration::from_millis(10));
    }
    // Now let the sync actor complete.
    future.unblock();
    handle.join().unwrap();
}

fn try_receive_next_actor<RT>(mut ctx: sync::Context<String, RT>) {
    loop {
        match ctx.try_receive_next() {
            Ok(msg) => {
                assert_eq!(msg, "Hello world");
                return;
            }
            Err(RecvError::Empty) => continue,
            Err(RecvError::Disconnected) => panic!("unexpected disconnected error"),
        }
    }
}

#[test]
fn context_try_receive_next() {
    let (handle, actor_ref) = spawn_sync_actor(
        NoSupervisor,
        actor_fn(try_receive_next_actor),
        (),
        SyncActorOptions::default(),
    )
    .unwrap();

    actor_ref.try_send("Hello world".to_owned()).unwrap();
    handle.join().unwrap();
}

#[test]
fn supervision() {
    let (handle, _) = spawn_sync_actor(
        bad_actor_supervisor,
        actor_fn(bad_actor),
        0usize,
        SyncActorOptions::default(),
    )
    .unwrap();

    handle.join().unwrap();
}

fn bad_actor_supervisor(err_count: usize) -> SupervisorStrategy<usize> {
    if err_count == 1 {
        SupervisorStrategy::Restart(err_count)
    } else {
        SupervisorStrategy::Stop
    }
}

fn bad_actor<RT>(_: sync::Context<!, RT>, count: usize) -> Result<(), usize> {
    Err(count + 1)
}
