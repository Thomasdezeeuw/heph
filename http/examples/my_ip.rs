#![feature(never_type)]

use std::borrow::Cow;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use heph::actor::{self, Actor, NewActor};
use heph::net::TcpStream;
use heph::rt::{self, Runtime, ThreadLocal};
use heph::spawn::options::{ActorOptions, Priority};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph::timer::Deadline;
use heph_http::body::OneshotBody;
use heph_http::{self as http, Header, HeaderName, Headers, HttpServer, Method, StatusCode};
use log::{debug, error, info, warn};

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
    let mut headers = Headers::EMPTY;
    loop {
        let fut = Deadline::after(&mut ctx, read_timeout, connection.next_request());
        let (code, body, should_close) = match fut.await? {
            Ok(Some(request)) => {
                info!("received request: {:?}: source={}", request, address);
                if request.path() != "/" {
                    (StatusCode::NOT_FOUND, "Not found".into(), false)
                } else if !matches!(request.method(), Method::Get | Method::Head) {
                    headers.append(Header::new(HeaderName::ALLOW, b"GET, HEAD"));
                    let body = "Method not allowed".into();
                    (StatusCode::METHOD_NOT_ALLOWED, body, false)
                } else if !request.body().is_empty() {
                    let body = Cow::from("Not expecting a body");
                    (StatusCode::PAYLOAD_TOO_LARGE, body, true)
                } else {
                    // This will allocate a new string which isn't the most
                    // efficient way to do this, but it's the easiest so we'll
                    // keep this for sake of example.
                    let body = Cow::from(address.ip().to_string());
                    (StatusCode::OK, body, false)
                }
            }
            // No more requests.
            Ok(None) => return Ok(()),
            Err(err) => {
                warn!("error reading request: {}: source={}", err, address);
                let code = err.proper_status_code();
                let body = Cow::from(format!("Bad request: {}", err));
                (code, body, err.should_close())
            }
        };

        if should_close {
            headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        }

        debug!(
            "sending response: code={}, body='{}', source={}",
            code, body, address
        );
        let body = OneshotBody::new(body.as_bytes());
        let write_response = connection.respond(code, &headers, body);
        Deadline::after(&mut ctx, WRITE_TIMEOUT, write_response).await?;

        if should_close {
            warn!("closing connection: source={}", address);
            return Ok(());
        }

        // Now that we've read a single request we can wait a little for the
        // next one so that we can reuse the resources for the next request.
        read_timeout = ALIVE_TIMEOUT;
        headers.clear();
    }
}
