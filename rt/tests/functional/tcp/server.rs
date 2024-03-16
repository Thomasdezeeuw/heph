use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::Duration;

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::messages::Terminate;
use heph::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use heph::ActorRef;
use heph_rt::net::{tcp, TcpStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{join_many, try_spawn_local, PanicSupervisor};
use heph_rt::{self as rt, Runtime, Signal, ThreadLocal};

use crate::util::{any_local_address, tcp_connect};

#[test]
fn message_from_terminate() {
    let _msg = tcp::server::Message::from(Terminate);
}

#[test]
fn message_from_process_signal() {
    let signals = &[Signal::Interrupt, Signal::Terminate, Signal::Quit];
    for signal in signals {
        assert!(tcp::server::Message::try_from(*signal).is_ok());
    }
}

async fn actor<RT>(_: actor::Context<!, RT>, stream: TcpStream)
where
    RT: rt::Access,
{
    let buf = Vec::with_capacity(DATA.len() + 1);
    let buf = stream.recv(buf).await.unwrap();
    assert_eq!(buf, DATA);
}

const DATA: &[u8] = b"Hello world";

async fn stream_actor<RT>(
    mut ctx: actor::Context<!, RT>,
    address: SocketAddr,
    actor_ref: ActorRef<tcp::server::Message>,
) where
    RT: rt::Access + Clone,
{
    let stream = tcp_connect(&mut ctx, address).await.unwrap();

    let (_, n) = stream.send(DATA).await.unwrap();
    assert_eq!(n, DATA.len());

    // Send a message to stop the listener.
    actor_ref.send(Terminate).await.unwrap();
}

#[test]
fn smoke() {
    let server = tcp::server::setup(
        any_local_address(),
        |err| panic!("unexpect error: {err}"),
        actor_fn(actor),
        ActorOptions::default(),
    )
    .unwrap();
    let server_address = server.local_addr();

    // TCP server should be able to be created outside the setup function and
    // used in it.
    let local_server = tcp::server::setup(
        any_local_address(),
        |err| panic!("unexpect error: {err}"),
        actor_fn(actor),
        ActorOptions::default(),
    )
    .unwrap();
    let mut runtime = Runtime::setup().build().unwrap();
    runtime
        .run_on_workers(move |mut runtime_ref| -> Result<(), !> {
            let server_address = local_server.local_addr();
            // Spawn thread-local version.
            let server_ref = runtime_ref
                .try_spawn_local(PanicSupervisor, local_server, (), ActorOptions::default())
                .unwrap();
            let _ = runtime_ref.spawn_local(
                NoSupervisor,
                actor_fn(stream_actor),
                (server_address, server_ref),
                ActorOptions::default(),
            );
            Ok(())
        })
        .unwrap();

    // Spawn thread-safe version.
    let server_ref = runtime
        .try_spawn(PanicSupervisor, server, (), ActorOptions::default())
        .unwrap();
    let _ = runtime.spawn(
        NoSupervisor,
        actor_fn(stream_actor),
        (server_address, server_ref),
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}

#[test]
fn zero_port() {
    let server = tcp::server::setup(
        any_local_address(),
        |err| panic!("unexpect error: {err}"),
        actor_fn::<_, _, ThreadLocal, _, _>(actor),
        ActorOptions::default(),
    )
    .unwrap();
    assert!(server.local_addr().port() != 0);
}

#[test]
fn new_actor_error() {
    struct ServerWrapper<T>(T);

    impl<NA> NewActor for ServerWrapper<NA>
    where
        NA: NewActor,
        ServerWrapper<NA::Actor>: Actor,
    {
        type Message = NA::Message;
        type Argument = NA::Argument;
        type Actor = ServerWrapper<NA::Actor>;
        type Error = NA::Error;
        type RuntimeAccess = NA::RuntimeAccess;

        fn new(
            &mut self,
            ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
            arg: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            self.0.new(ctx, arg).map(ServerWrapper)
        }
    }

    // NOTE: this is the whole point of the test, we need to get the `NewActor`
    // error here.
    impl<A> Actor for ServerWrapper<A>
    where
        A: Actor<Error = tcp::server::Error<()>>,
    {
        type Error = !;

        fn try_poll(
            self: Pin<&mut Self>,
            ctx: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            let res = Actor::try_poll(
                // SAFETY: not moving.
                unsafe { Pin::new_unchecked(&mut Pin::into_inner_unchecked(self).0) },
                ctx,
            );
            match res {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(tcp::server::Error::Accept(err))) => {
                    panic!("unexpected accept error: {err}")
                }
                Poll::Ready(Err(tcp::server::Error::NewActor(()))) => Poll::Ready(Ok(())),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    struct NewActorErrorGenerator<RT>(PhantomData<*const RT>);

    // `*const RT` is `!Send`, but we don't actually store it.
    unsafe impl<RT> Send for NewActorErrorGenerator<RT> {}
    unsafe impl<RT> Sync for NewActorErrorGenerator<RT> {}

    impl<RT> Copy for NewActorErrorGenerator<RT> {}

    impl<RT> Clone for NewActorErrorGenerator<RT> {
        fn clone(&self) -> Self {
            *self
        }
    }

    impl<RT> NewActor for NewActorErrorGenerator<RT>
    where
        RT: rt::Access,
    {
        type Message = !;
        type Argument = TcpStream;
        type Actor = ActorErrorGenerator;
        type Error = ();
        type RuntimeAccess = RT;

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::RuntimeAccess>,
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

    let server = tcp::server::setup(
        any_local_address(),
        ErrorSupervisor,
        NewActorErrorGenerator(PhantomData),
        ActorOptions::default(),
    )
    .unwrap();
    let address = server.local_addr();
    let server = ServerWrapper(server);
    let server_ref = try_spawn_local(ErrorSupervisor, server, (), ActorOptions::default()).unwrap();

    async fn stream_actor<M, RT>(mut ctx: actor::Context<M, RT>, address: SocketAddr)
    where
        RT: rt::Access + Clone,
    {
        let stream = tcp_connect(&mut ctx, address).await.unwrap();

        // Just need to create the connection.
        drop(stream);
    }

    let stream_actor = actor_fn(stream_actor);
    let stream_ref =
        try_spawn_local(NoSupervisor, stream_actor, address, ActorOptions::default()).unwrap();

    join_many(&[server_ref, stream_ref], Duration::from_secs(1)).unwrap();
}
