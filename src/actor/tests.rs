use std::any::Any;
use std::cell::Cell;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{self, Poll};

use crate::actor::{self, actor_fn, Actor, NewActor};
use crate::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use crate::ActorFuture;

#[test]
fn actor_name() {
    let tests = &[
        (
            "core::future::from_generator::GenFuture<1_hello_world::greeter_actor::{{closure}}>",
            "greeter_actor"
        ),
        (
            "core::future::from_generator::GenFuture<1_hello_world::some::nested::module::greeter::actor::{{closure}}>",
            "greeter::actor",
        ),
        (
            "core::future::from_generator::GenFuture<3_rpc::ping_actor>",
            "ping_actor"
        ),
        (
            "core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>",
            "conn_actor"
        ),
        (
            "2_my_ip::conn_actor::{{closure}}",
            "conn_actor",
        ),
        // Generic parameter(s) wrapped in GenFuture.
        (
            "core::future::from_generator::GenFuture<functional::functional::timer::triggered_timers_run_actors::deadline_actor<heph::actor::context::ThreadLocal>::{{closure}}>",
            "deadline_actor",
        ),
        (
            "core::future::from_generator::GenFuture<functional::functional::timer::triggered_timers_run_actors::timer_actor<heph::actor::context::ThreadSafe>::{{closure}}>",
            "timer_actor"
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1, P2>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A>, P2>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A>, P2<P2A>>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A<P1B>>, P2<P2A>>::{{closure}}>",
            "my_actor",
        ),
        // Async function named actor in an actor module.
        (
            "core::future::from_generator::GenFuture<heph::actor::tests::storage::actor::actor::{{closure}}>",
            "storage::actor",
        ),
        // Type implementing `Actor`.
        (
            "heph::net::tcp::server::TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context::ThreadLocal>",
            "TcpServer",
        ),
        // If the module path is removed from `std::any::type_name`.
        (
            "GenFuture<1_hello_world::greeter_actor::{{closure}}>",
            "greeter_actor"
        ),
        (
            "GenFuture<1_hello_world::some::nested::module::greeter::actor::{{closure}}>",
            "greeter::actor",
        ),
        (
            "TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context::ThreadLocal>",
            "TcpServer",
        ),
        (
            "TcpServer<core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>>",
            "TcpServer",
        ),
        (
            "TcpServer<GenFuture<2_my_ip::conn_actor::{{closure}}>>",
            "TcpServer",
        ),
    ];

    for (input, expected) in tests {
        let got = actor::format_name(input);
        assert_eq!(got, *expected, "input: {input}");
    }
}

async fn ok_actor(mut ctx: actor::Context<()>) {
    assert_eq!(ctx.receive_next().await, Ok(()));
}

#[test]
fn actor_future() {
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, actor_fn(ok_actor), ()).unwrap();
    let mut actor = pin!(actor);

    let (waker, count) = task_wake_counter();
    let mut ctx = task::Context::from_waker(&waker);

    // Actor should return `Poll::Pending` in the first call, since no message
    // is available.
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);

    // Send a message and the actor should return Ok.
    actor_ref.try_send(()).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Ready(()));
}

async fn error_actor(mut ctx: actor::Context<()>, fail: bool) -> Result<(), ()> {
    if fail {
        Err(())
    } else {
        assert_eq!(ctx.receive_next().await, Ok(()));
        Ok(())
    }
}

#[test]
fn erroneous_actor_process() {
    let mut supervisor_called_count = 0;
    let supervisor = |()| {
        supervisor_called_count += 1;
        SupervisorStrategy::Stop
    };
    let (actor, _) = ActorFuture::new(supervisor, actor_fn(error_actor), true).unwrap();
    let mut actor = pin!(actor);

    // Actor should return an error and be stopped.
    let (waker, count) = task_wake_counter();
    let mut ctx = task::Context::from_waker(&waker);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Ready(()));
    assert_eq!(supervisor_called_count, 1);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn restarting_erroneous_actor_process() {
    let supervisor_called_count = Cell::new(0);
    let supervisor = |()| {
        supervisor_called_count.set(supervisor_called_count.get() + 1);
        SupervisorStrategy::Restart(false)
    };
    let (actor, actor_ref) = ActorFuture::new(supervisor, actor_fn(error_actor), true).unwrap();
    let mut actor = pin!(actor);

    // Actor should return an error and be restarted.
    let (waker, count) = task_wake_counter();
    let mut ctx = task::Context::from_waker(&waker);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    assert_eq!(supervisor_called_count.get(), 1);
    // The future to wake itself after a restart to ensure it gets run again.
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // After a restart the actor should continue without issues.
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    assert_eq!(supervisor_called_count.get(), 1);

    // Finally after sending it a message it should complete.
    actor_ref.try_send(()).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Ready(()));
    assert_eq!(supervisor_called_count.get(), 1);
}

async fn panic_actor(mut ctx: actor::Context<()>, fail: bool) -> Result<(), ()> {
    if fail {
        panic!("oops!")
    } else {
        assert_eq!(ctx.receive_next().await, Ok(()));
        Ok(())
    }
}

#[test]
fn panicking_actor_process() {
    struct TestSupervisor<'a>(&'a mut usize);

    impl<NA> Supervisor<NA> for TestSupervisor<'_>
    where
        NA: NewActor<Argument = bool, Error = !>,
    {
        fn decide(&mut self, _: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
            unreachable!()
        }

        fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<NA::Argument> {
            // This can't be called.
            err
        }

        fn second_restart_error(&mut self, err: !) {
            // This can't be called.
            err
        }

        fn decide_on_panic(
            &mut self,
            panic: Box<dyn Any + Send + 'static>,
        ) -> SupervisorStrategy<NA::Argument> {
            drop(panic);
            *self.0 += 1;
            SupervisorStrategy::Stop
        }
    }

    let mut supervisor_called_count = 0;
    let supervisor = TestSupervisor(&mut supervisor_called_count);
    let (actor, _) = ActorFuture::new(supervisor, actor_fn(panic_actor), true).unwrap();
    let mut actor = pin!(actor);

    // Actor should panic and be stopped.
    let (waker, count) = task_wake_counter();
    let mut ctx = task::Context::from_waker(&waker);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Ready(()));
    assert_eq!(supervisor_called_count, 1);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn restarting_panicking_actor_process() {
    struct TestSupervisor<'a>(&'a Cell<usize>);

    impl<NA> Supervisor<NA> for TestSupervisor<'_>
    where
        NA: NewActor<Argument = bool, Error = !>,
    {
        fn decide(&mut self, _: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
            unreachable!()
        }

        fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<NA::Argument> {
            // This can't be called.
            err
        }

        fn second_restart_error(&mut self, err: !) {
            // This can't be called.
            err
        }

        fn decide_on_panic(
            &mut self,
            panic: Box<dyn Any + Send + 'static>,
        ) -> SupervisorStrategy<NA::Argument> {
            drop(panic);
            self.0.set(self.0.get() + 1);
            SupervisorStrategy::Restart(false)
        }
    }

    let supervisor_called_count = Cell::new(0);
    let supervisor = TestSupervisor(&supervisor_called_count);
    let (actor, actor_ref) = ActorFuture::new(supervisor, actor_fn(panic_actor), true).unwrap();
    let mut actor = pin!(actor);

    // Actor should panic and be restarted.
    let (waker, count) = task_wake_counter();
    let mut ctx = task::Context::from_waker(&waker);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    assert_eq!(supervisor_called_count.get(), 1);
    // The future to wake itself after a restart to ensure it gets run again.
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // After a restart the actor should continue without issues.
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    assert_eq!(supervisor_called_count.get(), 1);

    // Finally after sending it a message it should complete.
    actor_ref.try_send(()).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let res = actor.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Ready(()));
    assert_eq!(supervisor_called_count.get(), 1);
}

/// Returns a [`task::Waker`] that counts the times it's called in `call_count`.
pub(crate) fn task_wake_counter() -> (task::Waker, Arc<AtomicUsize>) {
    #[repr(transparent)]
    struct WakeCounter(AtomicUsize);

    impl task::Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            _ = self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    (
        // SAFETY: safe with `repr(transparent)`.
        task::Waker::from(unsafe {
            std::mem::transmute::<Arc<AtomicUsize>, Arc<WakeCounter>>(call_count.clone())
        }),
        call_count,
    )
}
