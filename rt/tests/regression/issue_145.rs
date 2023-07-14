//! The `TcpListener` and TCP server should bind to port 0, using the same port
//! on each worker thread.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::messages::Terminate;
use heph::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use heph::{Actor, ActorRef, NewActor};
use heph_rt::net::{tcp, TcpListener, TcpStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::{Runtime, RuntimeRef, ThreadLocal};

const N: usize = 4;

#[test]
fn issue_145_tcp_server() {
    let mut runtime = Runtime::setup().num_threads(N).build().unwrap();

    let addresses = Arc::new(Mutex::new(Vec::<SocketAddr>::new()));
    let servers = Arc::new(Mutex::new(Vec::new()));
    let addr2 = addresses.clone();
    let srv2 = servers.clone();
    let conn_actor =
        actor_fn(conn_actor).map_arg(move |stream| (stream, addr2.clone(), srv2.clone()));
    let address = "127.0.0.1:0".parse().unwrap();
    let server =
        tcp::server::setup(address, NoSupervisor, conn_actor, ActorOptions::default()).unwrap();
    let expected_address = server.local_addr();

    runtime
        .run_on_workers::<_, !>(move |mut runtime_ref| {
            let srv_ref =
                runtime_ref.spawn_local(ServerSupervisor, server, (), ActorOptions::default());
            // NOTE: this is not safe or supported. DO NOT USE THIS.
            let r = unsafe { std::mem::transmute_copy::<RuntimeRef, usize>(&runtime_ref) };
            servers.lock().unwrap().push((r, srv_ref));
            Ok(())
        })
        .unwrap();

    let handle = thread::spawn(move || {
        // TODO: replace with a barrier.
        thread::sleep(Duration::from_millis(50));

        for _ in 0..N {
            // Create a test connection to check the addresses.
            let mut stream = std::net::TcpStream::connect(&expected_address).unwrap();
            let mut buf = [0; 1];
            let n = stream.read(&mut buf).unwrap();
            assert_eq!(n, 0);

            // TODO: replace with a barrier.
            thread::sleep(Duration::from_millis(100));
        }
    });

    runtime.start().unwrap();

    handle.join().unwrap();
    for address in addresses.lock().unwrap().iter() {
        assert_eq!(*address, expected_address);
    }
}

struct ServerSupervisor;

impl<L, A> Supervisor<L> for ServerSupervisor
where
    L: NewActor<Message = tcp::server::Message, Argument = (), Actor = A, Error = !>,
    A: Actor<Error = tcp::server::Error<!>>,
{
    fn decide(&mut self, _: tcp::server::Error<!>) -> SupervisorStrategy<()> {
        SupervisorStrategy::Stop
    }

    fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<()> {
        err
    }

    fn second_restart_error(&mut self, err: !) {
        err
    }
}

#[allow(clippy::type_complexity)] // `servers` is too complex.
async fn conn_actor(
    mut ctx: actor::Context<!, ThreadLocal>,
    stream: TcpStream,
    addresses: Arc<Mutex<Vec<SocketAddr>>>,
    servers: Arc<Mutex<Vec<(usize, ActorRef<tcp::server::Message>)>>>,
) -> Result<(), !> {
    let mut addresses = addresses.lock().unwrap();
    addresses.push(stream.local_addr().unwrap());

    // Shutdown the TCP server that started us to ensure the next request goes
    // to a different server.
    // NOTE: this is not safe or supported. DO NOT USE THIS.
    let r = unsafe { std::mem::transmute_copy::<RuntimeRef, usize>(&*ctx.runtime()) };
    let mut servers = servers.lock().unwrap();
    let idx = servers.iter().position(|(rr, _)| r == *rr).unwrap();
    let (_, server_ref) = servers.remove(idx);
    server_ref.try_send(Terminate).unwrap();

    Ok(())
}

#[test]
fn issue_145_tcp_listener() {
    let mut runtime = Runtime::new().unwrap();
    runtime
        .run_on_workers::<_, !>(move |mut runtime_ref| {
            let actor = actor_fn(listener_actor);
            runtime_ref
                .try_spawn_local(NoSupervisor, actor, (), ActorOptions::default())
                .unwrap();
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}

async fn listener_actor(ctx: actor::Context<!, ThreadLocal>) -> Result<(), !> {
    let address = "127.0.0.1:0".parse().unwrap();
    // NOTE: this should not fail.
    let listener = TcpListener::bind(ctx.runtime_ref(), address).await.unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(addr.port() != 0);
    Ok(())
}
