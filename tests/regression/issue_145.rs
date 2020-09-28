//! The `TcpListener` and `tcp::Server` should bind to port 0, using the same
//! port on each worker thread.

use std::io::{self, Read};
use std::net::SocketAddr;
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use heph::actor::messages::Terminate;
use heph::net::tcp::{self, TcpListener, TcpStream};
use heph::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use heph::{actor, Actor, ActorOptions, ActorRef, NewActor, Runtime};

const N: usize = 4;

#[test]
fn issue_145_tcp_server() {
    let runtime = Runtime::new().unwrap();

    let addresses = Arc::new(Mutex::new(Vec::<SocketAddr>::new()));
    let address = "127.0.0.1:0".parse().unwrap();
    let addresses2 = addresses.clone();
    let conn_actor = (conn_actor as fn(_, _, _, _) -> _)
        .map_arg(move |(stream, address)| (stream, address, addresses2.clone()));
    let server =
        tcp::Server::setup(address, NoSupervisor, conn_actor, ActorOptions::default()).unwrap();
    let expected_address = server.local_addr();

    let shared = Arc::new((
        Mutex::new(()),
        Condvar::new(),
        Mutex::new(Option::<ActorRef<_>>::None),
        Condvar::new(),
        Barrier::new(2),
    ));

    // Start a thread that starts a single connection to `expected_address`.
    let sh = shared.clone();
    let handle = thread::spawn(move || {
        let (_, setup_cvar, server_ref, _, barrier) = &*sh;

        // TODO: replace with a barrier.
        thread::sleep(Duration::from_millis(100));

        for _ in 0..N {
            // Let a setup run.
            setup_cvar.notify_one();

            // Wait until the setup is complete so the worker thread is actually
            // running (and able to accept a connection).
            barrier.wait();

            // Create a test connection to check the addresses.
            let mut stream = std::net::TcpStream::connect(&expected_address).unwrap();
            let mut buf = [0; 1];
            let n = stream.read(&mut buf).unwrap();
            assert_eq!(n, 0);

            // Stop the server.
            server_ref
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(Terminate)
                .unwrap();
        }
    });

    runtime
        .num_threads(N)
        .with_setup::<_, !>(move |mut runtime_ref| {
            let (setup_lock, setup_cvar, server_ref, _, barrier) = &*shared;
            // By grabbing the `setup_lock` we block the other workers, so we
            // can ensure the thread above send a message the server we start.
            let setup_lock = setup_lock.lock().unwrap();
            let _g = setup_cvar.wait(setup_lock).unwrap();

            let options = ActorOptions::default();
            let srv_ref = runtime_ref
                .try_spawn_local(ServerSupervisor, server, (), options)
                .unwrap();
            assert!(server_ref.lock().unwrap().replace(srv_ref).is_none());

            // Let the thread start a connection to the server.
            barrier.wait();

            Ok(())
        })
        .start()
        .unwrap();

    handle.join().unwrap();
    for address in addresses.lock().unwrap().iter() {
        assert_eq!(*address, expected_address);
    }
}

struct ServerSupervisor;

impl<L, A> Supervisor<L> for ServerSupervisor
where
    L: NewActor<Message = tcp::ServerMessage, Argument = (), Actor = A, Error = io::Error>,
    A: Actor<Error = tcp::ServerError<!>>,
{
    fn decide(&mut self, _: tcp::ServerError<!>) -> SupervisorStrategy<()> {
        SupervisorStrategy::Stop
    }

    fn decide_on_restart_error(&mut self, _: io::Error) -> SupervisorStrategy<()> {
        SupervisorStrategy::Stop
    }

    fn second_restart_error(&mut self, _: io::Error) {}
}

async fn conn_actor(
    _: actor::Context<!>,
    mut stream: TcpStream,
    _: SocketAddr,
    addresses: Arc<Mutex<Vec<SocketAddr>>>,
) -> Result<(), !> {
    let mut addresses = addresses.lock().unwrap();
    addresses.push(stream.local_addr().unwrap());
    Ok(())
}

#[test]
fn issue_145_tcp_listener() {
    Runtime::new()
        .unwrap()
        .with_setup::<_, !>(move |mut runtime_ref| {
            let options = ActorOptions::default().mark_ready();
            let actor = listener_actor as fn(_) -> _;
            runtime_ref
                .try_spawn_local(NoSupervisor, actor, (), options)
                .unwrap();
            Ok(())
        })
        .start()
        .unwrap();
}

async fn listener_actor(mut ctx: actor::Context<!>) -> Result<(), !> {
    let address = "127.0.0.1:0".parse().unwrap();
    // NOTE: this should not fail.
    let mut listener = TcpListener::bind(&mut ctx, address).unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(addr.port() != 0);
    Ok(())
}
