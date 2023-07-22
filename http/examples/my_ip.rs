#![feature(never_type)]

use std::borrow::Cow;
use std::io;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::supervisor::SupervisorStrategy;
use heph_http::body::OneshotBody;
use heph_http::{self as http, server, Header, HeaderName, Headers, Method, StatusCode};
use heph_rt::net::TcpStream;
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::timer::Deadline;
use heph_rt::{Runtime, ThreadLocal};
use log::{debug, error, info, warn};

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
    let mut headers = Headers::EMPTY;
    loop {
        let fut = Deadline::after(
            ctx.runtime_ref().clone(),
            read_timeout,
            connection.next_request(),
        );
        let (code, body, should_close) = match fut.await {
            Ok(Some(request)) => {
                info!("received request: {request:?}: source={address}");
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
                warn!("error reading request: {err}: source={address}");
                let code = err.proper_status_code();
                let body = Cow::from(format!("Bad request: {err}"));
                (code, body, err.should_close())
            }
        };

        if should_close {
            headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        }

        debug!("sending response: code={code}, body='{body}', source={address}");
        let body = OneshotBody::new(body);
        let write_response = connection.respond(code, &headers, body);
        Deadline::after(ctx.runtime_ref().clone(), WRITE_TIMEOUT, write_response).await?;

        if should_close {
            warn!("closing connection: source={address}");
            return Ok(());
        }

        // Now that we've read a single request we can wait a little for the
        // next one so that we can reuse the resources for the next request.
        read_timeout = ALIVE_TIMEOUT;
        headers.clear();
    }
}
