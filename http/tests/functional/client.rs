#![allow(unused_imports)]

use std::borrow::Cow;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::Poll;
use std::thread::{self, sleep};
use std::time::{Duration, SystemTime};
use std::{fmt, str};

use heph::actor::{self, actor_fn};
use heph::messages::Terminate;
use heph::{Actor, ActorRef, NewActor, Supervisor, SupervisorStrategy};
use heph_http::body::{EmptyBody, OneshotBody};
use heph_http::client::{Client, ResponseError};
use heph_http::server::RequestError;
use heph_http::{self as http, Header, HeaderName, Headers, Method, Response, StatusCode, Version};
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::test::{init_actor, poll_actor};
use heph_rt::{Runtime, ThreadSafe};
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
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

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

#[test]
fn get_no_response() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client.get("/").await.unwrap_err();
            assert_eq!(err, ResponseError::IncompleteResponse);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // No response.
        drop(stream);

        handle.join().unwrap();
    });
}

#[test]
fn get_invalid_response() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client.get("/").await.unwrap_err();
            assert_eq!(err, ResponseError::InvalidStatus);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

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
            .write_all(b"HTTP/1.1 a00\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn request_with_headers() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let headers = Headers::from([Header::new(HeaderName::HOST, b"localhost")]);
            let response = client
                .request(Method::Get, "/", &headers, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([
                Header::new(HeaderName::USER_AGENT, USER_AGENT),
                Header::new(HeaderName::HOST, b"localhost"),
            ]),
            b"",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn request_with_user_agent_header() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let headers = Headers::from([Header::new(HeaderName::USER_AGENT, b"my-user-agent")]);
            let response = client
                .request(Method::Get, "/", &headers, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, b"my-user-agent")]),
            b"",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

/* FIXME: The following tests have the following problem:
 * error: higher-ranked lifetime error
 * = note: could not prove `impl Future<Output = Result<(), std::io::Error>>: Send`
#[test]
fn request_with_content_length_header() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let body = OneshotBody::new("Hi");
            // NOTE: Content-Length is incorrect for this test!
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"3")]);
            let response = client
                .request(Method::Get, "/", &headers, body)
                .await
                .unwrap();
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) = test_server.accept(|address| {
            init_actor(actor_fn(http_actor), address).unwrap().0

        });

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([
                Header::new(HeaderName::USER_AGENT, USER_AGENT),
                Header::new(HeaderName::CONTENT_LENGTH, b"3"),
            ]),
            b"hi",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn request_with_transfer_encoding_header() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let headers = Headers::from([Header::new(HeaderName::TRANSFER_ENCODING, b"identify")]);
            let body = OneshotBody::new("Hi");
            let response = client
                .request(Method::Get, "/", &headers, body)
                .await
                .unwrap();
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) = test_server.accept(|address| {
            init_actor(actor_fn(http_actor), address).unwrap().0

        });

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([
                Header::new(HeaderName::USER_AGENT, USER_AGENT),
                Header::new(HeaderName::TRANSFER_ENCODING, b"identify"),
            ]),
            b"hi",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn request_sets_content_length_header() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let body = OneshotBody::new("Ok");
            let response = client
                .request(Method::Get, "/", &Headers::EMPTY, body)
                .await
                .unwrap();
            let headers = Headers::from([Header::new(HeaderName::CONTENT_LENGTH, b"2")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) = test_server.accept(|address| {
            init_actor(actor_fn(http_actor), address).unwrap().0

        });

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([
                Header::new(HeaderName::USER_AGENT, USER_AGENT),
                Header::new(HeaderName::CONTENT_LENGTH, b"4"),
            ]),
            b"Hello",
        );

        // Write response.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}
*/

// TODO: add test with `ChunkedBody`.

#[test]
fn partial_response() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::IncompleteResponse);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Partal response, missing last `\r\n`.
        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\n")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn same_content_length() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([
                Header::new(HeaderName::CONTENT_LENGTH, b"2"),
                Header::new(HeaderName::CONTENT_LENGTH, b"2"),
            ]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn different_content_length() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::DifferentContentLengths);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\nContent-Length: 4\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn transfer_encoding_and_content_length_and() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::ContentLengthAndTransferEncoding);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_content_length() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidContentLength);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: abc\r\n\r\nOk")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn chunked_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(HeaderName::TRANSFER_ENCODING, b"chunked")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nOk0\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn slow_chunked_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(HeaderName::TRANSFER_ENCODING, b"chunked")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        sleep(Duration::from_millis(100));
        stream.write_all(b"2\r\nOk0\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn empty_chunked_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(HeaderName::TRANSFER_ENCODING, b"chunked")]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn content_length_and_identity_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([
                Header::new(HeaderName::CONTENT_LENGTH, b"2"),
                Header::new(HeaderName::TRANSFER_ENCODING, b"identity"),
            ]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(
                b"HTTP/1.1 200\r\nContent-Length: 2\r\nTransfer-Encoding: identity\r\n\r\nOk",
            )
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn unsupported_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::UnsupportedTransferEncoding);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: gzip\r\n\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn chunked_not_last_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client.get("/").await?;
            let headers = Headers::from([Header::new(
                HeaderName::TRANSFER_ENCODING,
                b"chunked, identity",
            )]);
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked, identity\r\n\r\nOk")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn content_length_and_transfer_encoding() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::ContentLengthAndTransferEncoding);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_chunk_size() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidChunkSize);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        stream
            .write_all(b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\nQ\r\nOk0\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn connect() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client
                .request(Method::Connect, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::EMPTY;
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Connect,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn head() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client
                .request(Method::Head, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::EMPTY;
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Head,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn response_status_204() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::EMPTY;
            let status = StatusCode::NO_CONTENT;
            expect_response(response, Version::Http11, status, &headers, b"").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 204\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn no_content_length_no_transfer_encoding_response() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let response = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap();
            let headers = Headers::EMPTY;
            expect_response(response, Version::Http11, StatusCode::OK, &headers, b"Ok").await;
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n\r\nOk").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn response_head_too_large() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::HeadTooLarge);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n").unwrap();
        let buf = [b'a'; http::MAX_HEAD_SIZE];
        stream.write_all(&buf).unwrap();
        stream.write_all(b"\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_header_name() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidHeaderName);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n\0: \r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_header_value() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidHeaderValue);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

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
            .write_all(b"HTTP/1.1 200\r\nAbc: Header\rvalue\r\n\r\n")
            .unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_new_line() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidNewLine);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"\rHTTP/1.1 200\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_version() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidVersion);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTPS/1.1 200\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn invalid_status() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::InvalidStatus);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 2009\r\n\r\n").unwrap();

        handle.join().unwrap();
    });
}

#[test]
fn too_many_headers() {
    with_test_server!(|test_server| {
        async fn http_actor(
            ctx: actor::Context<!, ThreadSafe>,
            address: SocketAddr,
        ) -> io::Result<()> {
            let mut client = Client::connect(ctx.runtime_ref(), address).await?;
            let err = client
                .request(Method::Get, "/", &Headers::EMPTY, EmptyBody)
                .await
                .unwrap_err();
            assert_eq!(err, ResponseError::TooManyHeaders);
            Ok(())
        }

        let (mut stream, handle) =
            test_server.accept(|address| init_actor(actor_fn(http_actor), address).unwrap().0);

        expect_request(
            &mut stream,
            Method::Get,
            "/",
            Version::Http11,
            &Headers::from([Header::new(HeaderName::USER_AGENT, USER_AGENT)]),
            b"",
        );

        // Write response.
        stream.write_all(b"HTTP/1.1 200\r\n").unwrap();
        for _ in 0..=http::MAX_HEADERS {
            stream.write_all(b"Some-Header: Abc\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();

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

    eprintln!("read request: {:?}", str::from_utf8(&buf[..n]));

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

async fn expect_response(
    mut response: Response<http::client::Body<'_>>,
    // Expected values:
    version: Version,
    status: StatusCode,
    headers: &Headers,
    body: &[u8],
) {
    eprintln!("read response: {response:?}");
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
        let expected = headers.get_bytes(&got_header.name()).unwrap();
        assert_eq!(
            got_header.value(),
            expected,
            "different header values for '{}' header, got: '{:?}', expected: '{:?}'",
            got_header.name(),
            str::from_utf8(got_header.value()),
            str::from_utf8(expected)
        );
    }
    let got_body = response
        .body_mut()
        .recv(Vec::with_capacity(1024))
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
        // TODO: don't run this on a different thread, use a test Heph runtime.
        let handle = thread::spawn(move || {
            for _ in 0..100 {
                match poll_actor(actor.as_mut()) {
                    Poll::Pending => {}
                    Poll::Ready(Ok(())) => return,
                    Poll::Ready(Err(err)) => panic!("error in actor: {err}"),
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
