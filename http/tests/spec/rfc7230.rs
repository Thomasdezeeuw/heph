//! Tests for RFC 7230 compliance.
//!
//! RFC 7230 is available at <https://datatracker.ietf.org/doc/html/rfc7230>.
//!
//! Applied errata: EID 4050, EID 4169, EID 4205, EID 4251, EID 4252, EID 4667,
//! EID 4825, EID 4839.

use std::io::{Read, Write};
use std::net::TcpStream;

#[test]
fn section_2_6_respond_with_highest_conformant_version() {
    // From RFC 7230 section 2.6:
    // > A server SHOULD send a response version equal to the highest version
    // > to which the server is conformant that has a major version less than
    // > or equal to the one received in the request.

    let address = spec_impl::start();
    let mut stream = TcpStream::connect(address).unwrap();
    // Send a HTTP/1.0 request.
    let req = b"GET / HTTP/1.0\r\n\r\n";
    stream.write_all(req).unwrap();
    // Expect a HTTP/1.1 response.
    let mut buf = [0; 128];
    let n = stream.read(&mut buf).unwrap();
    let expected = b"HTTP/1.1";
    assert!(n > expected.len());
    assert_eq!(&buf[..expected.len()], expected);
}

mod spec_impl {
    use std::borrow::Cow;
    use std::lazy::SyncLazy;
    use std::net::SocketAddr;
    use std::{io, thread};

    use heph::net::TcpStream;
    use heph::rt::ThreadLocal;
    use heph::spawn::options::{ActorOptions, Priority};
    use heph::{actor, Actor, NewActor, Runtime, Supervisor, SupervisorStrategy};
    use heph_http::body::OneshotBody;
    use heph_http::{self as http, Header, HeaderName, Headers, HttpServer, StatusCode};

    pub fn start() -> SocketAddr {
        *START_SERVER
    }

    static START_SERVER: SyncLazy<SocketAddr> = SyncLazy::new(|| {
        let actor = http_actor as fn(_, _, _) -> _;
        let address = "127.0.0.1:0".parse().unwrap();
        let server =
            HttpServer::setup(address, conn_supervisor, actor, ActorOptions::default()).unwrap();

        let address = server.local_addr();
        thread::spawn(move || {
            let mut runtime = Runtime::setup().num_threads(1).build().unwrap();
            runtime
                .run_on_workers(move |mut runtime_ref| -> io::Result<()> {
                    let options = ActorOptions::default().with_priority(Priority::LOW);
                    let server_ref = runtime_ref
                        .try_spawn_local(ServerSupervisor, server, (), options)
                        .unwrap();
                    runtime_ref.receive_signals(server_ref.try_map());
                    Ok(())
                })
                .unwrap();
            runtime.start().unwrap();
        });
        address
    });

    #[derive(Copy, Clone, Debug)]
    struct ServerSupervisor;

    impl<NA> Supervisor<NA> for ServerSupervisor
    where
        NA: NewActor<Argument = (), Error = io::Error>,
        NA::Actor: Actor<Error = http::server::Error<!>>,
    {
        fn decide(&mut self, err: http::server::Error<!>) -> SupervisorStrategy<()> {
            panic!("error accepting new connection: {}", err);
        }

        fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
            panic!("error restarting the TCP server: {}", err);
        }

        fn second_restart_error(&mut self, err: io::Error) {
            panic!("error restarting the actor a second time: {}", err);
        }
    }

    fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
        panic!("error handling connection: {}", err);
    }

    async fn http_actor(
        _: actor::Context<!, ThreadLocal>,
        mut connection: http::Connection,
        _: SocketAddr,
    ) -> io::Result<()> {
        connection.set_nodelay(true)?;

        let mut headers = Headers::EMPTY;
        loop {
            let (code, body, should_close) = match connection.next_request().await? {
                Ok(Some(_request)) => {
                    (StatusCode::OK, "".into(), false)
                    /*
                    if request.path() != "/" {
                        (StatusCode::NOT_FOUND, "Not found".into(), false)
                    } else if !matches!(request.method(), Method::Get | Method::Head) {
                        headers.add(Header::new(HeaderName::ALLOW, b"GET, HEAD"));
                        let body = "Method not allowed".into();
                        (StatusCode::METHOD_NOT_ALLOWED, body, false)
                    } else if request.body().len() != 0 {
                        let body = Cow::from("Not expecting a body");
                        (StatusCode::PAYLOAD_TOO_LARGE, body, true)
                    } else {
                        // This will allocate a new string which isn't the most
                        // efficient way to do this, but it's the easiest so we'll
                        // keep this for sake of example.
                        let body = Cow::from(address.ip().to_string());
                        (StatusCode::OK, body, false)
                    }
                    */
                }
                // No more requests.
                Ok(None) => return Ok(()),
                Err(err) => {
                    let code = err.proper_status_code();
                    let body = Cow::from(format!("Bad request: {}", err));
                    (code, body, err.should_close())
                }
            };

            if should_close {
                headers.add(Header::new(HeaderName::CONNECTION, b"close"));
            }

            let body = OneshotBody::new(body.as_bytes());
            connection.respond(code, &headers, body).await?;

            if should_close {
                return Ok(());
            }
            headers.clear();
        }
    }
}
