use std::borrow::Cow;
use std::io::{self, Read, Write};
use std::net::{self, Shutdown, SocketAddr};
use std::str;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, sleep};
use std::time::{Duration, SystemTime};

use heph::actor::{self, actor_fn};
use heph::messages::Terminate;
use heph::{ActorRef, SupervisorStrategy};
use heph_http::body::OneshotBody;
use heph_http::server::{self, RequestError};
use heph_http::{self as http, Header, HeaderName, Headers, Method, StatusCode, Version};
use heph_rt::net::TcpStream;
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::{self, Runtime, ThreadLocal};
use httpdate::fmt_http_date;

/// Macro to run with a test server.
macro_rules! with_test_server {
    (|$stream: ident| $test: block) => {
        let test_server = TestServer::spawn();
        // NOTE: we put `test` in a block to ensure all connections to the
        // server are dropped before we call `test_server.join()` below (which
        // would block a shutdown.
        {
            let mut $stream = loop {
                match net::TcpStream::connect(test_server.address) {
                    Ok(stream) => break stream,
                    Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {
                        // Give the server some time to start up.
                        sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(err) => panic!("failed to connect to {}: {err}", test_server.address),
                }
            };
            $stream.set_nodelay(true).unwrap();
            $stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            $stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            $test
        }
        test_server.join();
    };
}

#[test]
fn get() {
    with_test_server!(|stream| {
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"2"));
        let body = b"OK";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn head() {
    with_test_server!(|stream| {
        stream.write_all(b"HEAD / HTTP/1.1\r\n\r\n").unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"0"));
        let body = b"";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn post() {
    with_test_server!(|stream| {
        stream
            .write_all(b"POST /echo-body HTTP/1.1\r\nContent-Length: 11\r\n\r\nHello world")
            .unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"11"));
        let body = b"Hello world";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn with_request_header() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nUser-Agent:heph-http\r\n\r\n")
            .unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"2"));
        let body = b"OK";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn with_multiple_request_headers() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nUser-Agent:heph-http\r\nAccept: */*\r\n\r\n")
            .unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"2"));
        let body = b"OK";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn deny_incomplete_request() {
    with_test_server!(|stream| {
        // NOTE: missing `\r\n`.
        stream.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"31"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: incomplete request";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_unknown_method() {
    with_test_server!(|stream| {
        stream.write_all(b"MY_GET / HTTP/1.1\r\n\r\n").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::NOT_IMPLEMENTED;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"27"));
        let body = b"Bad request: unknown method";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_invalid_method() {
    with_test_server!(|stream| {
        stream.write_all(b"G\nE\rT / HTTP/1.1\r\n\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"35"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid request syntax";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn accept_same_content_length_headers() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"2"));
        let body = b"OK";
        expect_response(&mut stream, Version::Http11, StatusCode::OK, &headers, body);
    });
}

#[test]
fn deny_different_content_length_headers() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\nA")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"45"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: different Content-Length headers";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_content_length_and_chunked_transfer_encoding_request() {
    // NOTE: similar to
    // `deny_chunked_transfer_encoding_and_content_length_request`, but
    // Transfer-Encoding goes first.
    with_test_server!(|stream| {
        stream
            .write_all(
                b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\n\r\nA",
            )
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"71"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: provided both Content-Length and Transfer-Encoding headers";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_invalid_content_length_headers() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nContent-Length: ABC\r\n\r\n")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"42"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid Content-Length header";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn identity_transfer_encoding() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nTransfer-Encoding: identity\r\n\r\n")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::OK;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"2"));
        let body = b"OK";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn identity_transfer_encoding_with_content_length() {
    with_test_server!(|stream| {
        stream
            .write_all(
                b"POST /echo-body HTTP/1.1\r\nTransfer-Encoding: identity\r\nContent-Length: 1\r\n\r\nA",
            )
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::OK;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"1"));
        let body = b"A";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_unsupported_transfer_encoding() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nTransfer-Encoding: Nah\r\n\r\nA")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::NOT_IMPLEMENTED;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"42"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: unsupported Transfer-Encoding";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_empty_transfer_encoding() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nTransfer-Encoding:\r\n\r\n")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::NOT_IMPLEMENTED;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"42"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: unsupported Transfer-Encoding";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_chunked_transfer_encoding_not_last() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked, gzip\r\n\r\nA")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"45"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid Transfer-Encoding header";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_chunked_transfer_encoding_and_content_length_request() {
    // NOTE: similar to
    // `deny_content_length_and_chunked_transfer_encoding_request`, but
    // Content-Length goes first.
    with_test_server!(|stream| {
        stream
            .write_all(
                b"GET / HTTP/1.1\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\nA",
            )
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"71"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: provided both Content-Length and Transfer-Encoding headers";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn empty_body_chunked_transfer_encoding() {
    with_test_server!(|stream| {
        stream
            .write_all(b"POST /echo-body HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n")
            .unwrap();
        let status = StatusCode::OK;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"0"));
        let body = b"";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn deny_invalid_chunk_size() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nZ\r\nAbc0\r\n")
            .unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"31"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid chunk size";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn read_partial_chunk_size_chunked_transfer_encoding() {
    // Test `Connection::next_request` handling reading the HTTP head, but not
    // the chunk size yet.
    with_test_server!(|stream| {
        stream
            .write_all(b"POST /echo-body HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        sleep(Duration::from_millis(200));
        stream.write_all(b"0\r\n").unwrap();
        let status = StatusCode::OK;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"0"));
        let body = b"";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn too_large_http_head() {
    // Tests `heph_http::MAX_HEAD_SIZE`.
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nSOME_HEADER: ")
            .unwrap();
        let mut header_value = Vec::with_capacity(heph_http::MAX_HEAD_SIZE);
        header_value.resize(heph_http::MAX_HEAD_SIZE, b'a');
        stream.write_all(&header_value).unwrap();
        stream.write_all(b"\r\n\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"27"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: head too large";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn invalid_header_name() {
    with_test_server!(|stream| {
        stream.write_all(b"GET / HTTP/1.1\r\n\0: \r\n\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"32"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid header name";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn invalid_header_value() {
    with_test_server!(|stream| {
        stream
            .write_all(b"GET / HTTP/1.1\r\nAbc: Header\rvalue\r\n\r\n")
            .unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"33"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid header value";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn invalid_carriage_return() {
    with_test_server!(|stream| {
        stream.write_all(b"\rGET / HTTP/1.1\r\n\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"35"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid request syntax";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn invalid_http_version() {
    with_test_server!(|stream| {
        stream.write_all(b"GET / HTTPS/1.1\r\n\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"28"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: invalid version";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

#[test]
fn too_many_header() {
    with_test_server!(|stream| {
        stream.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        for _ in 0..=http::MAX_HEADERS {
            stream.write_all(b"Some-Header: Abc\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        let status = StatusCode::BAD_REQUEST;
        let mut headers = Headers::EMPTY;
        let now = fmt_http_date(SystemTime::now());
        headers.append(Header::new(HeaderName::DATE, now.as_bytes()));
        headers.append(Header::new(HeaderName::CONTENT_LENGTH, b"28"));
        headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        let body = b"Bad request: too many header";
        expect_response(&mut stream, Version::Http11, status, &headers, body);
    });
}

fn expect_response(
    stream: &mut net::TcpStream,
    // Expected values:
    version: Version,
    status: StatusCode,
    headers: &Headers,
    body: &[u8],
) {
    let mut buf = [0; 1024];
    let n = stream.read(&mut buf).unwrap();
    let buf = &buf[..n];

    eprintln!("read response: {:?}", str::from_utf8(&buf[..n]));

    let mut h = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut h);
    let parsed_n = response.parse(&buf).unwrap().unwrap();

    assert_eq!(response.version, Some(version.minor()));
    assert_eq!(response.code.unwrap(), status.0);
    assert!(response.reason.unwrap().is_empty()); // We don't send a reason-phrase.
    assert_eq!(
        response.headers.len(),
        headers.len(),
        "mismatch headers lengths, got: {:?}, expected: {:?}",
        response.headers,
        headers
    );
    for got_header in response.headers {
        let got_header_name = HeaderName::from_str(got_header.name);
        let got = headers.get_bytes(&got_header_name).unwrap();
        assert_eq!(
            got_header.value,
            got,
            "different header values for '{got_header_name}' header, got: '{:?}', expected: '{:?}'",
            str::from_utf8(got_header.value),
            str::from_utf8(got)
        );
    }
    assert_eq!(&buf[parsed_n..], body, "different bodies");
    assert_eq!(parsed_n, n - body.len(), "unexpected extra bytes");
}

struct TestServer {
    address: SocketAddr,
    server_ref: ActorRef<Terminate>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn spawn() -> Arc<TestServer> {
        static TEST_SERVER: Mutex<Weak<TestServer>> = Mutex::new(Weak::new());

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
        const TIMEOUT: Duration = Duration::from_secs(1);

        let server_ref = Arc::new((Mutex::new(None), Condvar::new()));
        let set_ref = server_ref.clone();

        let actor = actor_fn(http_actor);
        let address = "127.0.0.1:0".parse().unwrap();
        let server = server::setup(address, conn_supervisor, actor, ActorOptions::default())
            .map_err(heph_rt::Error::setup)
            .unwrap();
        let address = server.local_addr();

        let handle = thread::spawn(move || {
            let mut runtime = Runtime::setup().num_threads(1).build().unwrap();
            runtime
                .run_on_workers(move |mut runtime_ref| -> Result<(), !> {
                    let mut server_ref = set_ref.0.lock().unwrap();
                    let options = ActorOptions::default().with_priority(Priority::LOW);
                    *server_ref = Some(
                        runtime_ref
                            .try_spawn_local(server_supervisor, server, (), options)
                            .unwrap()
                            .map(),
                    );
                    set_ref.1.notify_all();
                    Ok(())
                })
                .unwrap();

            runtime.start().unwrap()
        });
        let mut server_ref = server_ref
            .1
            .wait_timeout_while(server_ref.0.lock().unwrap(), TIMEOUT, |r| r.is_none())
            .unwrap()
            .0;
        let server_ref = server_ref.take().unwrap();
        TestServer {
            address,
            server_ref,
            handle: Some(handle),
        }
    }

    fn join(mut self: Arc<TestServer>) {
        if let Some(this) = Arc::get_mut(&mut self) {
            this.server_ref.try_send(Terminate).unwrap();
            this.handle.take().unwrap().join().unwrap()
        }
    }
}

fn server_supervisor(err: http::server::Error<!>) -> SupervisorStrategy<()> {
    use http::server::Error::*;
    match err {
        Accept(err) => panic!("error accepting new connection: {err}"),
        NewActor(_) => unreachable!(),
    }
}

fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
    panic!("error handling connection: {err}")
}

/// Routes:
/// GET / => 200, OK.
/// POST /echo-body => 200, $request_body.
/// * => 404, Not found.
async fn http_actor(
    _: actor::Context<!, ThreadLocal>,
    mut connection: http::Connection,
) -> io::Result<()> {
    connection.set_nodelay(true)?;

    let mut headers = Headers::EMPTY;
    loop {
        let mut got_version = None;
        let mut got_method = None;
        let (code, body, should_close) = match connection.next_request().await {
            Ok(Some(mut request)) => {
                got_version = Some(request.version());
                got_method = Some(request.method());

                match (request.method(), request.path()) {
                    (Method::Get | Method::Head, "/") => (StatusCode::OK, "OK".into(), false),
                    (Method::Post, "/echo-body") => {
                        let body_len = request.body().len();
                        let buf = request.body_mut().recv(Vec::with_capacity(1024)).await?;
                        assert!(request.body().is_empty());
                        if let http::body::BodyLength::Known(length) = body_len {
                            assert_eq!(length, buf.len());
                        } else {
                            assert!(request.body().is_chunked());
                        }
                        let body = String::from_utf8(buf).unwrap().into();
                        (StatusCode::OK, body, false)
                    }
                    _ => (StatusCode::NOT_FOUND, "Not found".into(), false),
                }
            }
            // No more requests.
            Ok(None) => return Ok(()),
            Err(err) => {
                let code = err.proper_status_code();
                let body = Cow::from(format!("Bad request: {err}"));
                (code, body, err.should_close())
            }
        };
        if let Some(got_version) = got_version {
            assert_eq!(connection.last_request_version().unwrap(), got_version);
        }
        if let Some(got_method) = got_method {
            assert_eq!(connection.last_request_method().unwrap(), got_method);
        }

        if should_close {
            headers.append(Header::new(HeaderName::CONNECTION, b"close"));
        }

        connection
            .respond(code, &headers, OneshotBody::new(body))
            .await?;
        if should_close {
            return Ok(());
        }

        headers.clear();
    }
}

#[test]
fn request_error_proper_status_code() {
    use RequestError::*;
    let tests = &[
        (IncompleteRequest, StatusCode::BAD_REQUEST),
        (HeadTooLarge, StatusCode::BAD_REQUEST),
        (InvalidContentLength, StatusCode::BAD_REQUEST),
        (DifferentContentLengths, StatusCode::BAD_REQUEST),
        (InvalidHeaderName, StatusCode::BAD_REQUEST),
        (InvalidHeaderValue, StatusCode::BAD_REQUEST),
        (TooManyHeaders, StatusCode::BAD_REQUEST),
        (ChunkedNotLastTransferEncoding, StatusCode::BAD_REQUEST),
        (ContentLengthAndTransferEncoding, StatusCode::BAD_REQUEST),
        (InvalidToken, StatusCode::BAD_REQUEST),
        (InvalidNewLine, StatusCode::BAD_REQUEST),
        (InvalidVersion, StatusCode::BAD_REQUEST),
        (InvalidChunkSize, StatusCode::BAD_REQUEST),
        (UnsupportedTransferEncoding, StatusCode::NOT_IMPLEMENTED),
        (UnknownMethod, StatusCode::NOT_IMPLEMENTED),
    ];

    for (error, expected) in tests {
        assert_eq!(error.proper_status_code(), *expected);
    }
}

#[test]
fn request_should_close() {
    use RequestError::*;
    let tests = &[
        (IncompleteRequest, true),
        (HeadTooLarge, true),
        (InvalidContentLength, true),
        (DifferentContentLengths, true),
        (InvalidHeaderName, true),
        (InvalidHeaderValue, true),
        (UnsupportedTransferEncoding, true),
        (TooManyHeaders, true),
        (ChunkedNotLastTransferEncoding, true),
        (ContentLengthAndTransferEncoding, true),
        (InvalidToken, true),
        (InvalidNewLine, true),
        (InvalidVersion, true),
        (InvalidChunkSize, true),
        (UnknownMethod, false),
    ];

    for (error, expected) in tests {
        assert_eq!(error.should_close(), *expected);
    }
}
