#![cfg(feature = "test")]

use std::convert::TryFrom;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor::messages::Terminate;
use heph::actor::{self, Actor, NewActor, Spawn};
use heph::net::tcp::server;
use heph::net::{TcpServer, TcpStream};
use heph::rt::{self, ActorOptions, Signal};
use heph::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use heph::test::init_local_actor;
use heph::{ActorRef, Runtime};

use crate::util::{any_local_address, run_actors};

#[test]
fn message_from_terminate() {
    let _msg = server::Message::from(Terminate);
}

#[test]
fn message_from_process_signal() {
    let signals = &[Signal::Interrupt, Signal::Terminate, Signal::Quit];
    for signal in signals {
        assert!(server::Message::try_from(*signal).is_ok());
    }
}

#[derive(Copy, Clone)]
struct PanicSupervisor;

impl<S, NA> Supervisor<server::Setup<S, NA>> for PanicSupervisor
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !> + Clone + 'static,
    actor::Context<server::Message, <NA as NewActor>::Context>:
        rt::Access + Spawn<S, NA, NA::Context>,
    actor::Context<NA::Message, <NA as NewActor>::Context>: rt::Access,
{
    fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
        panic!("unexpected error: {}", err);
    }

    fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
        panic!("unexpected error: {}", err);
    }

    fn second_restart_error(&mut self, err: io::Error) {
        panic!("unexpected error: {}", err);
    }
}

async fn actor<K>(_: actor::Context<!, K>, mut stream: TcpStream, _: SocketAddr) -> Result<(), !>
where
    actor::Context<SocketAddr, K>: rt::Access,
{
    let mut buf = Vec::with_capacity(DATA.len() + 1);
    let n = stream.recv(&mut buf).await.unwrap();
    assert_eq!(n, DATA.len());
    assert_eq!(buf, DATA);
    Ok(())
}

const DATA: &[u8] = b"Hello world";

async fn stream_actor<K>(
    mut ctx: actor::Context<!, K>,
    address: SocketAddr,
    actor_ref: ActorRef<server::Message>,
) -> Result<(), !>
where
    actor::Context<!, K>: rt::Access,
{
    let mut stream = TcpStream::connect(&mut ctx, address)
        .unwrap()
        .await
        .unwrap();

    let n = stream.send(DATA).await.unwrap();
    assert_eq!(n, DATA.len());

    // Send a message to stop the listener.
    actor_ref.send(Terminate).await.unwrap();

    Ok(())
}

#[test]
fn smoke() {
    let server = TcpServer::setup(
        any_local_address(),
        |err| panic!("unexpect error: {}", err),
        actor as fn(_, _, _) -> _,
        ActorOptions::default(),
    )
    .unwrap();
    let server_address = server.local_addr();

    // `TcpServer` should be able to be created outside the setup function and
    // used in it.
    let local_server = TcpServer::setup(
        any_local_address(),
        |err| panic!("unexpect error: {}", err),
        actor as fn(_, _, _) -> _,
        ActorOptions::default(),
    )
    .unwrap();
    let mut runtime = Runtime::new()
        .unwrap()
        .with_setup(move |mut runtime_ref| -> Result<(), !> {
            let server_address = local_server.local_addr();
            // Spawn thread-local version.
            let server_ref = runtime_ref
                .try_spawn_local(PanicSupervisor, local_server, (), ActorOptions::default())
                .unwrap();
            let _ = runtime_ref.spawn_local(
                NoSupervisor,
                stream_actor as fn(_, _, _) -> _,
                (server_address, server_ref),
                ActorOptions::default(),
            );
            Ok(())
        });

    // Spawn thread-safe version.
    let server_ref = runtime
        .try_spawn(PanicSupervisor, server, (), ActorOptions::default())
        .unwrap();
    let _ = runtime.spawn(
        NoSupervisor,
        stream_actor as fn(_, _, _) -> _,
        (server_address, server_ref),
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}

#[test]
fn zero_port() {
    let actor = actor as fn(actor::Context<!>, _, _) -> _;
    let server = TcpServer::setup(
        any_local_address(),
        |err| panic!("unexpect error: {}", err),
        actor,
        ActorOptions::default(),
    )
    .unwrap();
    assert!(server.local_addr().port() != 0);
}

#[test]
fn new_actor_error() {
    struct ServerWrapper<S>(S);

    // NOTE: this is the whole point of the test, we need to get the `NewActor`
    // error here.
    impl<S> Actor for ServerWrapper<S>
    where
        S: Actor<Error = server::Error<()>>,
    {
        type Error = !;

        fn try_poll(
            self: Pin<&mut Self>,
            ctx: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            let res = Actor::try_poll(
                // Safety: not moving.
                unsafe { Pin::new_unchecked(&mut Pin::into_inner_unchecked(self).0) },
                ctx,
            );
            match res {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(server::Error::Accept(err))) => {
                    panic!("unexpected accept error: {}", err)
                }
                Poll::Ready(Err(server::Error::NewActor(()))) => Poll::Ready(Ok(())),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    struct NewActorErrorGenerator<K>(PhantomData<*const K>);

    impl<K> Copy for NewActorErrorGenerator<K> {}

    impl<K> Clone for NewActorErrorGenerator<K> {
        fn clone(&self) -> Self {
            *self
        }
    }

    impl<K> NewActor for NewActorErrorGenerator<K>
    where
        actor::Context<!, K>: rt::Access,
    {
        type Message = !;
        type Argument = (TcpStream, SocketAddr);
        type Actor = ActorErrorGenerator;
        type Error = ();
        type Context = K;

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::Context>,
            _: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            Err(())
        }
    }

    struct ActorErrorGenerator;

    impl Actor for ActorErrorGenerator {
        type Error = ();

        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Copy, Clone)]
    struct ErrorSupervisor;

    impl<NA> Supervisor<NA> for ErrorSupervisor
    where
        NA: NewActor,
    {
        fn decide(&mut self, _: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
            SupervisorStrategy::Stop
        }

        fn decide_on_restart_error(&mut self, _: NA::Error) -> SupervisorStrategy<NA::Argument> {
            SupervisorStrategy::Stop
        }

        fn second_restart_error(&mut self, _: NA::Error) {}
    }

    let server = TcpServer::setup(
        any_local_address(),
        ErrorSupervisor,
        NewActorErrorGenerator(PhantomData),
        ActorOptions::default(),
    )
    .unwrap();
    let address = server.local_addr();

    let (server_actor, _) = init_local_actor(server, ()).unwrap();
    let server_actor: Box<dyn Actor<Error = !>> = Box::new(ServerWrapper(server_actor));

    async fn stream_actor<K>(mut ctx: actor::Context<!, K>, address: SocketAddr) -> Result<(), !>
    where
        actor::Context<!, K>: rt::Access,
    {
        let stream = TcpStream::connect(&mut ctx, address)
            .unwrap()
            .await
            .unwrap();

        // Just need to create the connection.
        drop(stream);

        Ok(())
    }

    let stream_actor = stream_actor as fn(_, _) -> _;
    let (stream_actor, _) = init_local_actor(stream_actor, address).unwrap();
    let stream_actor: Box<dyn Actor<Error = !>> = Box::new(stream_actor);

    run_actors(vec![server_actor.into(), stream_actor.into()]);
}
