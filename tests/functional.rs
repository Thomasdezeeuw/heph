//! Functional tests.

#![feature(never_type, noop_waker)]

mod util {
    use std::future::Future;
    use std::mem::size_of;
    use std::pin::pin;
    use std::task::{self, Poll};

    pub fn assert_send<T: Send>() {}

    pub fn assert_sync<T: Sync>() {}

    #[track_caller]
    pub fn assert_size<T>(expected: usize) {
        assert_eq!(size_of::<T>(), expected);
    }

    pub fn block_on<Fut: Future>(fut: Fut) -> Fut::Output {
        let mut fut = pin!(fut);
        let mut ctx = task::Context::from_waker(task::Waker::noop());
        loop {
            match fut.as_mut().poll(&mut ctx) {
                Poll::Ready(output) => return output,
                Poll::Pending => {}
            }
        }
    }
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor;
    mod actor_group;
    mod actor_ref;
    mod restart_supervisor;
    mod sync_actor;
    mod test;
}
