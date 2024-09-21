//! Functional tests.

#![feature(never_type, noop_waker)]

mod util {
    use std::future::Future;
    use std::pin::{pin, Pin};
    use std::task::{self, Poll};

    use heph::{actor, sync, Actor, NewActor, SyncActor};

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
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor;
    mod actor_group;
    mod actor_ref;
    mod restart_supervisor;
    mod stop_supervisor;
    mod sync_actor;
    mod test;
}
