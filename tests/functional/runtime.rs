use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{self, Poll};
use std::thread::{self, sleep};
use std::time::Duration;

use heph::actor::{self, SyncContext};
use heph::rt::options::{ActorOptions, Priority, SyncActorOptions};
use heph::rt::{Runtime, ThreadLocal, ThreadSafe};
use heph::supervisor::NoSupervisor;

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

    use heph::actor::messages::Terminate;
    use heph::net::tcp::server;
    use heph::net::{TcpServer, TcpStream};
    use heph::rt::{ActorOptions, RuntimeRef, ThreadLocal};
    use heph::supervisor::{Supervisor, SupervisorStrategy};
    use heph::{actor, ActorRef, NewActor};

    fn cpu_affinity(stream: &TcpStream) -> io::Result<usize> {
        // TODO: do this better.
        let socket =
            SockRef::from(unsafe { &*(stream as *const TcpStream as *const mio::net::TcpStream) });
        socket.cpu_affinity()
    }

    async fn stream_actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        address: SocketAddr,
        server_ref: ActorRef<server::Message>,
    ) -> io::Result<()> {
        let stream = TcpStream::connect(&mut ctx, address)?.await?;

        let cpu = cpu_affinity(&stream).unwrap();
        assert_eq!(cpu, 0);

        server_ref.send(Terminate).await.unwrap();
        Ok(())
    }

    async fn accepted_stream_actor(
        _: actor::Context<!, ThreadLocal>,
        stream: TcpStream,
        _: SocketAddr,
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

    impl<S, NA> Supervisor<server::Setup<S, NA>> for ServerSupervisor
    where
        S: Supervisor<NA> + Clone + 'static,
        NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, RuntimeAccess = ThreadLocal>
            + Clone
            + 'static,
    {
        fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
            panic!("unexpected error accept stream: {}", err);
        }

        fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
            panic!("unexpected restarting server: {}", err);
        }

        fn second_restart_error(&mut self, err: io::Error) {
            panic!("unexpected restarting server: {}", err);
        }
    }

    fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
        // Check the CPU affinity of the worker thread.
        check_thread_affinity(0)?;

        let address = "127.0.0.1:0".parse().unwrap();
        let accepted_stream_actor = accepted_stream_actor as fn(_, TcpStream, SocketAddr) -> _;
        let server = TcpServer::setup(
            address,
            |err: io::Error| panic!("unexpected error: {}", err),
            accepted_stream_actor,
            ActorOptions::default(),
        )?;
        let address = server.local_addr();
        let server_ref =
            runtime_ref.try_spawn_local(ServerSupervisor, server, (), ActorOptions::default())?;

        let stream_actor = stream_actor as fn(_, _, _) -> _;
        let args = (address, server_ref);
        let _ = runtime_ref.spawn_local(
            |err| panic!("unexpected error: {}", err),
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
        .current_dir("./tools")
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
                actor as fn(_, _) -> _,
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
        actor as fn(_, _) -> _,
        future,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
    handle.join().unwrap();
}

#[test]
fn external_thread_wakes_sync_actor() {
    fn actor(mut ctx: SyncContext<!>, future: WaitFuture) -> Result<(), !> {
        ctx.block_on(future)
    }

    let (future, handle) = WaitFuture::new();

    let mut runtime = Runtime::new().unwrap();
    let _ = runtime.spawn_sync_actor(
        NoSupervisor,
        actor as fn(_, _) -> _,
        future,
        SyncActorOptions::default(),
    );

    runtime.start().unwrap();
    handle.join().unwrap();
}
