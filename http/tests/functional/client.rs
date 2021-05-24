#![allow(unused_imports)]

use std::borrow::Cow;
use std::io::{self, Read, Write};
use std::lazy::SyncLazy;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::Poll;
use std::thread::{self, sleep};
use std::time::{Duration, SystemTime};
use std::{fmt, str};

use heph::actor::messages::Terminate;
use heph::rt::{self, Runtime, ThreadSafe};
use heph::spawn::options::{ActorOptions, Priority};
use heph::test::{init_actor, poll_actor};
use heph::{actor, Actor, ActorRef, NewActor, Supervisor, SupervisorStrategy};
use heph_http::body::OneshotBody;
use heph_http::server::{HttpServer, RequestError};
use heph_http::{
    self as http, Client, Header, HeaderName, Headers, Method, Response, StatusCode, Version,
};
use httpdate::fmt_http_date;

const USER_AGENT: &[u8] = b"Heph-HTTP/0.1.0";

/// Macro to run with a test server.
macro_rules! with_test_server {
    (|$test_server: ident| $test: block) => {
        let test_server = TestServer::spawn();
        let $test_server = test_server;
        $test
    };
}

#[test]
fn get() {
    with_test_server!(|test_server| {
        async fn http_actor(
            mut ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(&mut ctx, address)?.await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) = test_server.accept(|address| {
            let http_actor = http_actor as fn(_, _) -> _;
            let (actor, _) = init_actor(http_actor, address).unwrap();
            actor
        });

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

fn expect_request(
    stream: &mut TcpStream,
    // Expected values:
    method: Method,
    path: &str,
    version: Version,
    headers: &Headers,
    body: &[u8],
) {
    let mut buf = [0; 1024];
    let n = stream.read(&mut buf).unwrap();
    let buf = &buf[..n];

    eprintln!("read response: {:?}", str::from_utf8(&buf[..n]));

    let mut h = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut h);
    let parsed_n = request.parse(&buf).unwrap().unwrap();

    assert_eq!(request.method, Some(method.as_str()));
    assert_eq!(request.path, Some(path));
    assert_eq!(request.version, Some(version.minor()));
    assert_eq!(
        request.headers.len(),
        headers.len(),
        "mismatch headers lengths, got: {:?}, expected: {:?}",
        request.headers,
        headers
    );
    for got_header in request.headers {
        let got_header_name = HeaderName::from_str(got_header.name);
        let got = headers.get_value(&got_header_name).unwrap();
        assert_eq!(
            got_header.value,
            got,
            "different header values for '{}' header, got: '{:?}', expected: '{:?}'",
            got_header_name,
            str::from_utf8(got_header.value),
            str::from_utf8(got)
        );
    }
    assert_eq!(&buf[parsed_n..], body, "different bodies");
    assert_eq!(parsed_n, n - body.len(), "unexpected extra bytes");
}

async fn expect_response(
    mut response: Response<http::client::Body<'_>>,
    // Expected values:
    version: Version,
    status: StatusCode,
    headers: &Headers,
    body: &[u8],
) {
    eprintln!("read response: {:?}", response);
    assert_eq!(response.version(), version);
    assert_eq!(response.status(), status);
    assert_eq!(
        response.headers().len(),
        headers.len(),
        "mismatch headers lengths, got: {:?}, expected: {:?}",
        response.headers(),
        headers
    );
    for got_header in response.headers().iter() {
        let expected = headers.get_value(&got_header.name()).unwrap();
        assert_eq!(
            got_header.value(),
            expected,
            "different header values for '{}' header, got: '{:?}', expected: '{:?}'",
            got_header.name(),
            str::from_utf8(got_header.value()),
            str::from_utf8(expected)
        );
    }
    let mut got_body = Vec::new();
    response
        .body_mut()
        .read_all(&mut got_body, 1024)
        .await
        .unwrap();
    assert_eq!(got_body, body, "different bodies");
}

struct TestServer {
    address: SocketAddr,
    listener: Mutex<TcpListener>,
}

impl TestServer {
    fn spawn() -> Arc<TestServer> {
        static TEST_SERVER: SyncLazy<Mutex<Weak<TestServer>>> =
            SyncLazy::new(|| Mutex::new(Weak::new()));

        let mut test_server = TEST_SERVER.lock().unwrap();
        if let Some(test_server) = test_server.upgrade() {
            // Use an existing running server.
            test_server
        } else {
            // Start a new server.
            let new_server = Arc::new(TestServer::new());
            *test_server = Arc::downgrade(&new_server);
            new_server
        }
    }

    fn new() -> TestServer {
        let address: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = TcpListener::bind(address).unwrap();
        let address = listener.local_addr().unwrap();

        TestServer {
            address,
            listener: Mutex::new(listener),
        }
    }

    #[track_caller]
    fn accept<F, A>(&self, spawn: F) -> (TcpStream, thread::JoinHandle<()>)
    where
        F: FnOnce(SocketAddr) -> A,
        A: Actor + Send + 'static,
        A::Error: fmt::Display,
    {
        let listener = self.listener.lock().unwrap();
        let actor = spawn(self.address);
        let mut actor = Box::pin(actor);
        let handle = thread::spawn(move || {
            for _ in 0..100 {
                match poll_actor(actor.as_mut()) {
                    Poll::Pending => {}
                    Poll::Ready(Ok(())) => return,
                    Poll::Ready(Err(err)) => panic!("error in actor: {}", err),
                }
                sleep(Duration::from_millis(10));
            }
            panic!("looped too many times");
        });
        let (stream, _) = listener.accept().unwrap();
        drop(listener);
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        (stream, handle)
    }
}
