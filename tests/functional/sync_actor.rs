#![cfg(feature = "test")]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::actor::sync::SyncContext;
use heph::rt::SyncActorOptions;
use heph::supervisor::NoSupervisor;
use heph::test::spawn_sync_actor;

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

fn block_on_actor<Fut>(mut ctx: SyncContext<String>, fut: Fut)
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
        block_on_actor as fn(_, _) -> _,
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
