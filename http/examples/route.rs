#![feature(never_type)]

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use heph::actor::{self, Actor, NewActor};
use heph::net::TcpStream;
use heph::rt::{self, Runtime, ThreadLocal};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph::timer::Deadline;
use heph_http::body::OneshotBody;
use heph_http::{self as http, route, HttpServer, Request, Response};
use heph_rt::spawn::options::{ActorOptions, Priority};
use log::{error, info, warn};

fn main() -> Result<(), rt::Error> {
    std_logger::init();

    let actor = http_actor as fn(_, _, _) -> _;
    let address = "127.0.0.1:7890".parse().unwrap();
    let server = HttpServer::setup(address, conn_supervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    let mut runtime = Runtime::setup().use_all_cores().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;

        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;
    info!("listening on http://{}", address);
    runtime.start()
}

/// Our supervisor for the TCP server.
#[derive(Copy, Clone, Debug)]
struct ServerSupervisor;

impl<NA> Supervisor<NA> for ServerSupervisor
where
    NA: NewActor<Argument = (), Error = io::Error>,
    NA::Actor: Actor<Error = http::server::Error<!>>,
{
    fn decide(&mut self, err: http::server::Error<!>) -> SupervisorStrategy<()> {
        use http::server::Error::*;
        match err {
            Accept(err) => {
                error!("error accepting new connection: {}", err);
                SupervisorStrategy::Restart(())
            }
            NewActor(_) => unreachable!(),
        }
    }

    fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
        error!("error restarting the TCP server: {}", err);
        SupervisorStrategy::Stop
    }

    fn second_restart_error(&mut self, err: io::Error) {
        error!("error restarting the actor a second time: {}", err);
    }
}

fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}

const READ_TIMEOUT: Duration = Duration::from_secs(10);
const ALIVE_TIMEOUT: Duration = Duration::from_secs(120);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

async fn http_actor(
    mut ctx: actor::Context<!, ThreadLocal>,
    mut connection: http::Connection,
    address: SocketAddr,
) -> io::Result<()> {
    info!("accepted connection: source={}", address);
    connection.set_nodelay(true)?;

    let mut read_timeout = READ_TIMEOUT;
    loop {
        let fut = Deadline::after(&mut ctx, read_timeout, connection.next_request());

        let response = match fut.await? {
            Ok(Some(request)) => {
                info!("received request: {:?}: source={}", request, address);
                route!(match request {
                    GET | HEAD "/" => index,
                    GET | HEAD "/other_page" => other_page,
                    POST       "/post" => post,
                    _ => not_found,
                })
            }
            // No more requests.
            Ok(None) => return Ok(()),
            Err(err) => {
                warn!("error reading request: {}: source={}", err, address);
                err.response().with_body("Bad request".into())
            }
        };

        let write_response = connection.respond_with(response);
        Deadline::after(&mut ctx, WRITE_TIMEOUT, write_response).await?;

        // Now that we've read a single request we can wait a little for the
        // next one so that we can reuse the resources for the next request.
        read_timeout = ALIVE_TIMEOUT;
    }
}

async fn index<B>(_req: Request<B>) -> Response<OneshotBody<'static>> {
    Response::ok().with_body("Index".into())
}

async fn other_page<B>(_req: Request<B>) -> Response<OneshotBody<'static>> {
    Response::ok().with_body("Other page!".into())
}

async fn post<B>(_req: Request<B>) -> Response<OneshotBody<'static>> {
    Response::ok().with_body("POST".into())
}

async fn not_found<B>(_req: Request<B>) -> Response<OneshotBody<'static>> {
    Response::not_found().with_body("Page not found".into())
}
