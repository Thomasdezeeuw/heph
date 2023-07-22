#![feature(never_type)]

use std::io;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::supervisor::SupervisorStrategy;
use heph_http::body::OneshotBody;
use heph_http::{self as http, route, server, Request, Response};
use heph_rt::net::TcpStream;
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::timer::Deadline;
use heph_rt::{Runtime, ThreadLocal};
use log::{error, info, warn};

fn main() -> Result<(), heph_rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    let actor = actor_fn(http_actor);
    let address = "127.0.0.1:7890".parse().unwrap();
    let server = server::setup(address, conn_supervisor, actor, ActorOptions::default())
        .map_err(heph_rt::Error::setup)?;

    let mut runtime = Runtime::setup().use_all_cores().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.spawn_local(server_supervisor, server, (), options);

        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;
    info!("listening on http://{address}");
    runtime.start()
}

fn server_supervisor(err: server::Error<!>) -> SupervisorStrategy<()> {
    match err {
        // When we hit an error accepting a connection we'll drop the old
        // server and create a new one.
        server::Error::Accept(err) => {
            error!("error accepting new connection: {err}");
            SupervisorStrategy::Restart(())
        }
        // Async function never return an error creating a new actor.
        server::Error::NewActor(_) => unreachable!(),
    }
}

fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
    error!("error handling connection: {err}");
    SupervisorStrategy::Stop
}

const READ_TIMEOUT: Duration = Duration::from_secs(10);
const ALIVE_TIMEOUT: Duration = Duration::from_secs(120);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

async fn http_actor(
    ctx: actor::Context<!, ThreadLocal>,
    mut connection: http::Connection,
) -> io::Result<()> {
    let address = connection.peer_addr()?;
    info!("accepted connection: source={address}");
    connection.set_nodelay(true)?;

    let mut read_timeout = READ_TIMEOUT;
    loop {
        let fut = Deadline::after(
            ctx.runtime_ref().clone(),
            read_timeout,
            connection.next_request(),
        );

        let response = match fut.await {
            Ok(Some(request)) => {
                info!("received request: {request:?}: source={address}");
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
                warn!("error reading request: {err}: source={address}");
                err.response().with_body(OneshotBody::new("Bad request"))
            }
        };

        let write_response = connection.respond_with(response);
        Deadline::after(ctx.runtime_ref().clone(), WRITE_TIMEOUT, write_response).await?;

        // Now that we've read a single request we can wait a little for the
        // next one so that we can reuse the resources for the next request.
        read_timeout = ALIVE_TIMEOUT;
    }
}

async fn index<B>(_req: Request<B>) -> Response<OneshotBody<&'static str>> {
    Response::ok().with_body(OneshotBody::new("Index"))
}

async fn other_page<B>(_req: Request<B>) -> Response<OneshotBody<&'static str>> {
    Response::ok().with_body(OneshotBody::new("Other page!"))
}

async fn post<B>(_req: Request<B>) -> Response<OneshotBody<&'static str>> {
    Response::ok().with_body(OneshotBody::new("POST"))
}

async fn not_found<B>(_req: Request<B>) -> Response<OneshotBody<&'static str>> {
    Response::not_found().with_body(OneshotBody::new("Page not found"))
}
