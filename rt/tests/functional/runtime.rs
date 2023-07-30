use std::future::Future;
use std::io::{self, Write};
use std::marker::PhantomData;
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{self, Poll};
use std::thread::{self, sleep};
use std::time::Duration;

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use heph::sync;
use heph_rt::spawn::options::{ActorOptions, FutureOptions, Priority, SyncActorOptions};
use heph_rt::{Runtime, ThreadLocal, ThreadSafe};

use crate::util::temp_file;

#[test]
#[allow(clippy::eq_op)] // Need to compare `Priority` to itself.
fn priority() {
    assert!(Priority::HIGH > Priority::NORMAL);
    assert!(Priority::NORMAL > Priority::LOW);
    assert!(Priority::HIGH > Priority::LOW);

    assert_eq!(Priority::HIGH, Priority::HIGH);
    assert_ne!(Priority::HIGH, Priority::NORMAL);

    assert_eq!(Priority::default(), Priority::NORMAL);
}

#[test]
#[cfg(target_os = "linux")] // Only works on Linux.
fn auto_cpu_affinity() {
    use std::net::SocketAddr;

    use socket2::SockRef;

    use heph::messages::Terminate;
    use heph::supervisor::{Supervisor, SupervisorStrategy};
    use heph::{actor, ActorRef, NewActor};
    use heph_rt::net::{tcp, TcpStream};
    use heph_rt::spawn::ActorOptions;
    use heph_rt::{RuntimeRef, ThreadLocal};

    use crate::util::tcp_connect;

    fn cpu_affinity(stream: &TcpStream) -> io::Result<usize> {
        // TODO: do this better.
        let socket =
            SockRef::from(unsafe { &*(stream as *const TcpStream as *const a10::AsyncFd) });
        socket.cpu_affinity()
    }

    async fn stream_actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        address: SocketAddr,
        server_ref: ActorRef<tcp::server::Message>,
    ) -> io::Result<()> {
        let stream = tcp_connect(&mut ctx, address).await?;

        let cpu = cpu_affinity(&stream).unwrap();
        assert_eq!(cpu, 0);

        server_ref.send(Terminate).await.unwrap();
        Ok(())
    }

    async fn accepted_stream_actor(
        _: actor::Context<!, ThreadLocal>,
        stream: TcpStream,
    ) -> io::Result<()> {
        let cpu = cpu_affinity(&stream)?;
        assert_eq!(cpu, 0);
        Ok(())
    }

    fn check_thread_affinity(cpu: usize) -> io::Result<()> {
        use std::mem;
        let mut cpu_set: libc::cpu_set_t = unsafe { mem::zeroed() };
        unsafe { libc::CPU_ZERO(&mut cpu_set) };
        let thread = unsafe { libc::pthread_self() };
        let res = unsafe {
            libc::pthread_getaffinity_np(thread, mem::size_of_val(&cpu_set), &mut cpu_set)
        };
        if res != 0 {
            return Err(io::Error::last_os_error());
        }
        assert!(unsafe { libc::CPU_ISSET(cpu, &cpu_set) });
        Ok(())
    }

    /// Our supervisor for the TCP server.
    #[derive(Copy, Clone, Debug)]
    struct ServerSupervisor;

    impl<S, NA> Supervisor<tcp::server::Setup<S, NA>> for ServerSupervisor
    where
        S: Supervisor<NA> + Clone + 'static,
        NA: NewActor<Argument = TcpStream, Error = !, RuntimeAccess = ThreadLocal>
            + Clone
            + 'static,
    {
        fn decide(&mut self, err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
            panic!("unexpected error accept stream: {err}");
        }

        fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<()> {
            err
        }

        fn second_restart_error(&mut self, err: !) {
            err
        }
    }

    fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
        // Check the CPU affinity of the worker thread.
        check_thread_affinity(0)?;

        let address = "127.0.0.1:0".parse().unwrap();
        let accepted_stream_actor = actor_fn(accepted_stream_actor);
        let server = tcp::server::setup(
            address,
            |err: io::Error| panic!("unexpected error: {err}"),
            accepted_stream_actor,
            ActorOptions::default(),
        )?;
        let address = server.local_addr();
        let server_ref =
            runtime_ref.spawn_local(ServerSupervisor, server, (), ActorOptions::default());

        let stream_actor = actor_fn(stream_actor);
        let args = (address, server_ref);
        let _ = runtime_ref.spawn_local(
            |err| panic!("unexpected error: {err}"),
            stream_actor,
            args,
            ActorOptions::default(),
        );

        Ok(())
    }

    let mut runtime = Runtime::setup().auto_cpu_affinity().build().unwrap();
    runtime.run_on_workers(setup).unwrap();
    runtime.start().unwrap();
}

#[test]
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn running_actors() {
    use SupervisorStrategy::*;

    #[rustfmt::skip]
    fn tests<RT>() -> Vec<(
        RunningSupervisor<Result<(), ()>>,
        RunningNewActor<RT>,
        Result<(), ()>,
        StatusCheck,
    )> {
        vec![
            // Actor: Ok.
            test(Ok(()), vec![], vec![], 0, 0, 0, 0, 0),
            // Actor: Err, supervisor: stop.
            test(Err(()), vec![Stop], vec![], 1, 0, 0, 0, 1),
            // Actor: Err, supervisor: restart, new_actor: ok.
            test(Err(()), vec![Restart(Ok(()))], vec![true], 1, 0, 0, 1, 0),
            // Actor: Err, supervisor: restart, new_actor: err, supervisor: stop.
            test(Err(()), vec![Restart(Ok(())), Stop], vec![false], 1, 1, 0, 1, 1),
            // Actor: Err, supervisor: restart, new_actor: err, supervisor: restart.
            test(Err(()), vec![Restart(Ok(())), Restart(Ok(()))], vec![false, true], 1, 1, 0, 2, 0),
            // Actor: Err, supervisor: restart, new_actor: err, supervisor: restart, new_actor: err.
            test(Err(()), vec![Restart(Ok(())), Restart(Ok(()))], vec![false, false], 1, 1, 1, 2, 0),
        ]
    }

    let mut runtime = Runtime::setup().build().unwrap();

    let local_status = Arc::new(Mutex::new(Vec::new()));
    let s = local_status.clone();
    runtime
        .run_on_workers(move |mut runtime_ref| -> Result<(), !> {
            let mut local_status = Vec::new();
            for (supervisor, new_actor, arg, status) in tests() {
                runtime_ref
                    .try_spawn_local(supervisor, new_actor, arg, ActorOptions::default())
                    .unwrap();
                local_status.push(status);
            }
            s.lock().unwrap().append(&mut local_status);
            Ok(())
        })
        .unwrap();

    let mut safe_status = Vec::new();
    for (supervisor, new_actor, arg, status) in tests() {
        runtime
            .try_spawn(supervisor, new_actor, arg, ActorOptions::default())
            .unwrap();
        safe_status.push(status);
    }

    runtime.start().unwrap();

    for (i, status) in local_status.lock().unwrap().iter().enumerate() {
        eprintln!("thread-local {i}. status: {status:?}");
        status.assert();
    }

    for (i, status) in safe_status.into_iter().enumerate() {
        eprintln!("thread-safe {i}. status: {status:?}");
        status.assert();
    }

    fn test<RT>(
        initial_arg: Result<(), ()>,
        mut restarts: Vec<SupervisorStrategy<Result<(), ()>>>,
        mut new_actor_results: Vec<bool>,
        // Status checks:
        actor_errors: usize,
        restart_errors: usize,
        second_restart_errors: usize,
        restarted: usize,
        stopped: usize,
    ) -> (
        RunningSupervisor<Result<(), ()>>,
        RunningNewActor<RT>,
        Result<(), ()>,
        StatusCheck,
    ) {
        let status = Arc::new(RestartStatus {
            actor_errors: AtomicUsize::new(0),
            restart_errors: AtomicUsize::new(0),
            second_restart_errors: AtomicUsize::new(0),
            restarted: AtomicUsize::new(0),
            stopped: AtomicUsize::new(0),
        });
        restarts.reverse();
        let supervisor = RunningSupervisor {
            restarts,
            status: status.clone(),
        };
        new_actor_results.reverse();
        new_actor_results.push(true); // First creation.
        let new_actor = RunningNewActor(new_actor_results, PhantomData);
        let check_status = StatusCheck(
            status,
            actor_errors,
            restart_errors,
            second_restart_errors,
            restarted,
            stopped,
        );
        (supervisor, new_actor, initial_arg, check_status)
    }

    #[derive(Clone)]
    struct RunningNewActor<RT>(Vec<bool>, PhantomData<RT>);

    // Don't actually own a `RT`, so it's OK.
    unsafe impl<RT> Send for RunningNewActor<RT> {}

    impl<RT> NewActor for RunningNewActor<RT> {
        type Message = ();
        type Argument = Result<(), ()>;
        type Actor = RunningActor;
        type Error = ();
        type RuntimeAccess = RT;

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::RuntimeAccess>,
            arg: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            if self.0.pop().unwrap() {
                Ok(RunningActor(arg))
            } else {
                Err(())
            }
        }
    }

    #[derive(Clone)]
    struct RunningActor(Result<(), ()>);

    impl Actor for RunningActor {
        type Error = ();

        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(self.0)
        }
    }

    #[derive(Debug)]
    struct StatusCheck(Arc<RestartStatus>, usize, usize, usize, usize, usize);

    #[derive(Debug)]
    struct RestartStatus {
        actor_errors: AtomicUsize,
        restart_errors: AtomicUsize,
        second_restart_errors: AtomicUsize,
        restarted: AtomicUsize,
        stopped: AtomicUsize,
    }

    impl StatusCheck {
        fn assert(&self) {
            assert_eq!(get(&self.0.actor_errors), self.1);
            assert_eq!(get(&self.0.restart_errors), self.2);
            assert_eq!(get(&self.0.second_restart_errors), self.3);
            assert_eq!(get(&self.0.restarted), self.4);
            assert_eq!(get(&self.0.stopped), self.5);
        }
    }

    #[derive(Clone, Debug)] // `Clone` for `run_on_workers`.
    struct RunningSupervisor<T> {
        restarts: Vec<SupervisorStrategy<T>>,
        status: Arc<RestartStatus>,
    }

    fn get(value: &AtomicUsize) -> usize {
        value.load(Ordering::SeqCst)
    }

    fn incr(value: &AtomicUsize) {
        let _ = value.fetch_add(1, Ordering::SeqCst);
    }

    impl<NA> Supervisor<NA> for RunningSupervisor<NA::Argument>
    where
        NA: NewActor,
    {
        fn decide(&mut self, _: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
            incr(&self.status.actor_errors);
            let ret = self.restarts.pop().unwrap();
            if let Stop = ret {
                incr(&self.status.stopped);
            } else {
                incr(&self.status.restarted);
            }
            ret
        }

        fn decide_on_restart_error(&mut self, _: NA::Error) -> SupervisorStrategy<NA::Argument> {
            incr(&self.status.restart_errors);
            let ret = self.restarts.pop().unwrap();
            if let Stop = ret {
                incr(&self.status.stopped);
            } else {
                incr(&self.status.restarted);
            }
            ret
        }

        fn second_restart_error(&mut self, _: NA::Error) {
            incr(&self.status.second_restart_errors);
        }
    }
}

#[test]
fn tracing() {
    let trace_path = temp_file("runtime_trace.bin.trace");
    let output_path = temp_file("runtime_trace.json");

    // Generate a simple trace.
    let mut setup = Runtime::setup();
    setup.enable_tracing(&trace_path).unwrap();
    setup.build().unwrap().start().unwrap();

    // Convert the trace just to make sure it's valid.
    let output = Command::new("cargo")
        .arg("run")
        .arg("--bin")
        .arg("convert_trace")
        .arg(trace_path)
        .arg(output_path)
        .current_dir("../tools")
        .output()
        .expect("failed to convert trace");
    if !output.status.success() {
        let stderr = io::stderr();
        let mut stderr = stderr.lock();
        stderr
            .write_all(b"Failed to convert trace\nStandard out:")
            .unwrap();
        stderr.write_all(&output.stdout).unwrap();
        stderr.write_all(b"\nStandard err:").unwrap();
        stderr.write_all(&output.stderr).unwrap();
    }
}

#[derive(Clone)] // Needed in setup function.
struct WaitFuture {
    #[allow(clippy::type_complexity)]
    inner: Arc<(Mutex<(Option<task::Waker>, bool)>, Condvar)>,
}

impl Future for WaitFuture {
    type Output = Result<(), !>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut guard = self.inner.0.lock().unwrap();
        match &mut *guard {
            (_, true) => Poll::Ready(Ok(())),
            (waker, false) => {
                *waker = Some(ctx.waker().clone());
                self.inner.1.notify_all();
                Poll::Pending
            }
        }
    }
}

impl WaitFuture {
    fn new() -> (WaitFuture, thread::JoinHandle<()>) {
        let inner = Arc::new((Mutex::new((None, false)), Condvar::new()));
        let future = WaitFuture {
            inner: inner.clone(),
        };

        let handle = thread::spawn(move || {
            let mut guard = inner.0.lock().unwrap();
            loop {
                match &mut *guard {
                    (Some(waker), called) => {
                        // Ensure the worker thread is sleeping.
                        sleep(Duration::from_millis(100));

                        waker.wake_by_ref();
                        *called = true;
                        break;
                    }
                    (None, _) => {
                        guard = inner.1.wait(guard).unwrap();
                    }
                }
            }
        });
        (future, handle)
    }
}

#[test]
fn external_thread_wakes_thread_local_actor() {
    async fn actor(_: actor::Context<!, ThreadLocal>, future: WaitFuture) -> Result<(), !> {
        future.await
    }

    let (future, handle) = WaitFuture::new();

    let mut runtime = Runtime::setup().build().unwrap();
    runtime
        .run_on_workers::<_, !>(|mut runtime_ref| {
            let _ = runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(actor),
                future,
                ActorOptions::default(),
            );
            Ok(())
        })
        .unwrap();

    runtime.start().unwrap();
    handle.join().unwrap();
}

#[test]
fn external_thread_wakes_thread_safe_actor() {
    async fn actor(_: actor::Context<!, ThreadSafe>, future: WaitFuture) -> Result<(), !> {
        future.await
    }

    let (future, handle) = WaitFuture::new();

    let mut runtime = Runtime::new().unwrap();
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(actor),
        future,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
    handle.join().unwrap();
}

#[test]
fn external_thread_wakes_sync_actor() {
    fn actor<RT>(mut ctx: sync::Context<!, RT>, future: WaitFuture) -> Result<(), !> {
        ctx.block_on(future)
    }

    let (future, handle) = WaitFuture::new();

    let mut runtime = Runtime::new().unwrap();
    let _ = runtime.spawn_sync_actor(
        NoSupervisor,
        actor_fn(actor),
        future,
        SyncActorOptions::default(),
    );

    runtime.start().unwrap();
    handle.join().unwrap();
}

async fn panic_actor<RT>(_: actor::Context<!, RT>, mark: &'static AtomicBool) {
    mark.store(true, Ordering::SeqCst);
    panic!("on purpose panic");
}

async fn ok_actor<RT>(_: actor::Context<!, RT>, mark: &'static AtomicBool) {
    mark.store(true, Ordering::SeqCst);
}

fn actor_drop_panic<RT>(_: actor::Context<!, RT>, mark: &'static AtomicBool) -> PanicOnDropFuture {
    PanicOnDropFuture(mark)
}

struct PanicOnDropFuture(&'static AtomicBool);

impl Future for PanicOnDropFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        self.0.store(true, Ordering::SeqCst);
        Poll::Ready(())
    }
}

impl Drop for PanicOnDropFuture {
    fn drop(&mut self) {
        panic!("on purpose panic");
    }
}

async fn panic_future(mark: &'static AtomicBool) {
    mark.store(true, Ordering::SeqCst);
    panic!("on purpose panic");
}

async fn ok_future(mark: &'static AtomicBool) {
    mark.store(true, Ordering::SeqCst);
}

#[test]
fn catches_actor_panics() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime.spawn(
        NoSupervisor,
        actor_fn(panic_actor),
        &PANIC_RAN,
        ActorOptions::default().with_priority(Priority::HIGH),
    );
    runtime.spawn(
        NoSupervisor,
        actor_fn(ok_actor),
        &OK_RAN,
        ActorOptions::default().with_priority(Priority::LOW),
    );
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_local_actor_panics() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers(|mut runtime_ref| -> Result<(), !> {
            runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(panic_actor),
                &PANIC_RAN,
                ActorOptions::default().with_priority(Priority::HIGH),
            );
            runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(ok_actor),
                &OK_RAN,
                ActorOptions::default().with_priority(Priority::LOW),
            );
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_actor_panics_on_drop() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime.spawn(
        NoSupervisor,
        actor_fn(actor_drop_panic),
        &PANIC_RAN,
        ActorOptions::default().with_priority(Priority::HIGH),
    );
    runtime.spawn(
        NoSupervisor,
        actor_fn(ok_actor),
        &OK_RAN,
        ActorOptions::default().with_priority(Priority::LOW),
    );
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_local_actor_panics_on_drop() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers(|mut runtime_ref| -> Result<(), !> {
            runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(actor_drop_panic),
                &PANIC_RAN,
                ActorOptions::default().with_priority(Priority::HIGH),
            );
            runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(ok_actor),
                &OK_RAN,
                ActorOptions::default().with_priority(Priority::LOW),
            );
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_future_panics() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime.spawn_future(
        panic_future(&PANIC_RAN),
        FutureOptions::default().with_priority(Priority::HIGH),
    );
    runtime.spawn_future(
        ok_future(&OK_RAN),
        FutureOptions::default().with_priority(Priority::LOW),
    );
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_local_future_panics() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers(|mut runtime_ref| -> Result<(), !> {
            runtime_ref.spawn_local_future(
                panic_future(&PANIC_RAN),
                FutureOptions::default().with_priority(Priority::HIGH),
            );
            runtime_ref.spawn_local_future(
                ok_future(&OK_RAN),
                FutureOptions::default().with_priority(Priority::LOW),
            );
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_future_panics_on_drop() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime.spawn_future(
        PanicOnDropFuture(&PANIC_RAN),
        FutureOptions::default().with_priority(Priority::HIGH),
    );
    runtime.spawn_future(
        ok_future(&OK_RAN),
        FutureOptions::default().with_priority(Priority::LOW),
    );
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}

#[test]
fn catches_local_future_panics_on_drop() {
    static PANIC_RAN: AtomicBool = AtomicBool::new(false);
    static OK_RAN: AtomicBool = AtomicBool::new(false);

    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers(|mut runtime_ref| -> Result<(), !> {
            runtime_ref.spawn_local_future(
                PanicOnDropFuture(&PANIC_RAN),
                FutureOptions::default().with_priority(Priority::HIGH),
            );
            runtime_ref.spawn_local_future(
                ok_future(&OK_RAN),
                FutureOptions::default().with_priority(Priority::LOW),
            );
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();

    assert!(PANIC_RAN.load(Ordering::SeqCst));
    assert!(OK_RAN.load(Ordering::SeqCst));
}
