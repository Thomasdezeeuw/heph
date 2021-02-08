use std::io::{self, Write};

use std::process::Command;

use heph::rt::options::Priority;
use heph::rt::Runtime;

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

    use heph::actor::context::ThreadLocal;
    use heph::actor::messages::Terminate;
    use heph::net::tcp::server;
    use heph::net::{TcpServer, TcpStream};
    use heph::supervisor::{Supervisor, SupervisorStrategy};
    use heph::{actor, ActorOptions, ActorRef, NewActor, RuntimeRef};

    fn cpu_affinity(stream: &TcpStream) -> io::Result<usize> {
        // TODO: do this better.
        let socket =
            SockRef::from(unsafe { &*(stream as *const TcpStream as *const mio::net::TcpStream) });
        socket.cpu_affinity()
    }

    async fn stream_actor(
        mut ctx: actor::Context<!>,
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
        _: actor::Context<!>,
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
        NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = ThreadLocal>
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
        let accepted_stream_actor =
            accepted_stream_actor as fn(actor::Context<!>, TcpStream, SocketAddr) -> _;
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

    let runtime = Runtime::new().unwrap();
    runtime
        .with_setup(setup)
        .auto_cpu_affinity()
        .start()
        .unwrap();
}

#[test]
fn tracing() {
    let trace_path = temp_file("runtime_trace.bin.trace");
    let output_path = temp_file("runtime_trace.json");

    // Generate a simple trace.
    let mut runtime = Runtime::new().unwrap();
    runtime.enable_tracing(&trace_path).unwrap();
    runtime.start().unwrap();

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
