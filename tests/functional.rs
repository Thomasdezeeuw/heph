//! Functional tests.

#![feature(never_type)]

mod util {
    use std::future::Future;
    use std::pin::{Pin, pin};
    use std::task::{self, Poll};

    use heph::{Actor, NewActor, SyncActor, actor, sync};

    pub(crate) fn assert_send<T: Send>() {}

    pub(crate) fn assert_sync<T: Sync>() {}

    #[track_caller]
    pub(crate) fn assert_size<T>(expected: usize) {
        assert_eq!(size_of::<T>(), expected);
    }

    pub(crate) fn block_on<Fut: Future>(fut: Fut) -> Fut::Output {
        let mut fut = pin!(fut);
        let mut ctx = task::Context::from_waker(task::Waker::noop());
        loop {
            match fut.as_mut().poll(&mut ctx) {
                Poll::Ready(output) => return output,
                Poll::Pending => {}
            }
        }
    }

    pub(crate) fn block_on_many(mut futs: Vec<Pin<Box<dyn Future<Output = ()>>>>) {
        let mut ctx = task::Context::from_waker(task::Waker::noop());
        while !futs.is_empty() {
            futs.extract_if(.., |fut| match fut.as_mut().poll(&mut ctx) {
                Poll::Ready(()) => true,
                Poll::Pending => false,
            })
            .for_each(drop)
        }
    }

    pub(crate) fn poll<Fut: Future>(fut: Pin<&mut Fut>) -> Poll<Fut::Output> {
        let mut ctx = task::Context::from_waker(task::Waker::noop());
        fut.poll(&mut ctx)
    }

    pub(crate) fn poll_once<Fut: Future>(fut: Pin<&mut Fut>) {
        let mut ctx = task::Context::from_waker(task::Waker::noop());
        match fut.poll(&mut ctx) {
            Poll::Ready(_) => panic!("unexpected output"),
            Poll::Pending => {}
        }
    }

    pub(crate) struct EmptyNewActor;

    impl NewActor for EmptyNewActor {
        type Message = !;
        type Argument = ();
        type Actor = EmptyActor;
        type Error = &'static str;
        type RuntimeAccess = ();

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::RuntimeAccess>,
            _: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            Ok(EmptyActor)
        }
    }

    pub(crate) struct EmptyActor;

    impl Actor for EmptyActor {
        type Error = &'static str;
        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    impl SyncActor for EmptyActor {
        type Message = !;
        type Argument = ();
        type Error = &'static str;
        type RuntimeAccess = ();

        fn run(
            &self,
            _: sync::Context<Self::Message, Self::RuntimeAccess>,
            _: Self::Argument,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// Returns a [`Future`] that return [`Poll::Pending`] once, without waking
    /// itself.
    pub(crate) const fn pending_once() -> PendingOnce {
        PendingOnce(false)
    }

    pub(crate) struct PendingOnce(bool);

    impl Future for PendingOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                ctx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor;
    mod actor_context;
    mod actor_group;
    mod actor_ref;
    mod from_message;
    mod restart_supervisor;
    mod stop_supervisor;
    mod sync_actor;
    mod test;
}
