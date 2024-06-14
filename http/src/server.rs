// TODO: Continue reading RFC 7230 section 4 Transfer Codings.
//
// TODO: RFC 7230 section 3.3.3 point 5:
// > If the sender closes the connection or the recipient
// > times out before the indicated number of octets are
// > received, the recipient MUST consider the message to be
// > incomplete and close the connection.

//! HTTP server.
//!
//! The HTTP server is an actor that starts a new actor for each accepted HTTP
//! connection. This actor can start as a thread-local or thread-safe actor.
//! When using the thread-local variant one actor runs per worker thread which
//! spawns thread-local actors to handle the [`Connection`]s, from which HTTP
//! [`Request`]s can be read and HTTP [`Response`]s can be written.
//!
//! [`Response`]: crate::Response
//!
//! # Graceful shutdown
//!
//! Graceful shutdown is done by sending it a [`Terminate`] message. The HTTP
//! server can also handle (shutdown) process signals, see below for an example.
//!
//! [`Terminate`]: heph::messages::Terminate
//!
//! # Examples
//!
//! ```rust
//! # #![feature(never_type)]
//! use std::borrow::Cow;
//! use std::io;
//! use std::net::SocketAddr;
//! use std::time::Duration;
//!
//! use heph::actor::{self, Actor, NewActor, actor_fn};
//! use heph::supervisor::{Supervisor, SupervisorStrategy};
//! use heph_http::body::OneshotBody;
//! use heph_http::{self as http, server, Header, HeaderName, Headers, Method, StatusCode};
//! use heph_rt::net::TcpStream;
//! use heph_rt::spawn::options::{ActorOptions, Priority};
//! use heph_rt::timer::Deadline;
//! use heph_rt::{Runtime, ThreadLocal};
//! use log::error;
//!
//! fn main() -> Result<(), heph_rt::Error> {
//!     // Setup the HTTP server.
//!     let actor = actor_fn(http_actor);
//!     let address = "127.0.0.1:7890".parse().unwrap();
//!     let server = server::setup(address, conn_supervisor, actor, ActorOptions::default())
//!         .map_err(heph_rt::Error::setup)?;
//!
//!     // Build the runtime.
//!     let mut runtime = Runtime::setup().use_all_cores().build()?;
//!     // On each worker thread start our HTTP server.
//!     runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
//!         let options = ActorOptions::default().with_priority(Priority::LOW);
//!         let server_ref = runtime_ref.spawn_local(server_supervisor, server, (), options);
//!
//! #       server_ref.try_send(heph::messages::Terminate).unwrap();
//!
//!         // Allow graceful shutdown by responding to process signals.
//!         runtime_ref.receive_signals(server_ref.try_map());
//!         Ok(())
//!     })?;
//!     runtime.start()
//! }
//!
//! /// Our supervisor for the HTTP server.
//! fn server_supervisor(err: server::Error<!>) -> SupervisorStrategy<()> {
//!     match err {
//!         // When we hit an error accepting a connection we'll drop the old
//!         // server and create a new one.
//!         server::Error::Accept(err) => {
//!             error!("error accepting new connection: {err}");
//!             SupervisorStrategy::Restart(())
//!         }
//!         // Async function never return an error creating a new actor.
//!         server::Error::NewActor(_) => unreachable!(),
//!     }
//! }
//!
//! fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
//!     error!("error handling connection: {err}");
//!     SupervisorStrategy::Stop
//! }
//!
//! /// Our actor that handles a single HTTP connection.
//! async fn http_actor(
//!     mut ctx: actor::Context<!, ThreadLocal>,
//!     mut connection: http::Connection,
//! ) -> io::Result<()> {
//!     // Set `TCP_NODELAY` on the underlying `TcpStream`.
//!     connection.set_nodelay(true)?;
//!
//!     let mut headers = Headers::EMPTY;
//!     loop {
//!         // Read the next request.
//!         let (code, body, should_close) = match connection.next_request().await {
//!             Ok(Some(request)) => {
//!                 // Only support GET/HEAD to "/", with an empty body.
//!                 if request.path() != "/" {
//!                     (StatusCode::NOT_FOUND, "Not found".into(), false)
//!                 } else if !matches!(request.method(), Method::Get | Method::Head) {
//!                     // Add the "Allow" header to show the HTTP methods we do
//!                     // support.
//!                     headers.append(Header::new(HeaderName::ALLOW, b"GET, HEAD"));
//!                     (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed".into(), false)
//!                 } else if !request.body().is_empty() {
//!                     (StatusCode::PAYLOAD_TOO_LARGE, "Not expecting a body".into(), true)
//!                 } else {
//!                     (StatusCode::OK, "Hello world".into(), false)
//!                 }
//!             }
//!             // No more requests.
//!             Ok(None) => return Ok(()),
//!             // Error parsing request.
//!             Err(err) => {
//!                 // Determine the correct status code to return.
//!                 let code = err.proper_status_code();
//!                 // Create a useful error message as body.
//!                 let body = Cow::from(format!("Bad request: {err}"));
//!                 (code, body, err.should_close())
//!             }
//!         };
//!
//!         // If we want to close the connection add the "Connection: close"
//!         // header.
//!         if should_close {
//!             headers.append(Header::new(HeaderName::CONNECTION, b"close"));
//!         }
//!
//!         // Send the body as a single payload.
//!         let body = OneshotBody::new(body);
//!         // Respond to the request.
//!         connection.respond(code, &headers, body).await?;
//!
//!         if should_close {
//!             return Ok(());
//!         }
//!         headers.clear();
//!     }
//! }
//! ```

use std::fmt;
use std::io::{self, Write};
use std::mem::{take, MaybeUninit};
use std::net::SocketAddr;
use std::time::SystemTime;

use heph::{actor, NewActor, Supervisor};
use heph_rt::io::{BufMut, BufMutSlice};
use heph_rt::net::{tcp, TcpStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::timer::DeadlinePassed;
use httpdate::HttpDate;

use crate::body::{BodyLength, EmptyBody};
use crate::head::header::{FromHeaderValue, Header, HeaderName, Headers};
use crate::{
    map_version_byte, trim_ws, Method, Request, Response, StatusCode, Version, BUF_SIZE,
    INIT_HEAD_SIZE, MAX_HEADERS, MAX_HEAD_SIZE, MIN_READ_SIZE,
};

/// Create a new [server setup].
///
/// Arguments:
///  * `address`: the address to listen on.
///  * `supervisor`: the [`Supervisor`] used to supervise each started actor,
///  * `new_actor`: the [`NewActor`] implementation to start each actor, and
///  * `options`: the actor options used to spawn the new actors.
///
/// See the [module documentation] for examples.
///
/// [server setup]: Setup
/// [module documentation]: crate::server
pub fn setup<S, NA>(
    address: SocketAddr,
    supervisor: S,
    new_actor: NA,
    options: ActorOptions,
) -> io::Result<Setup<S, NA>>
where
    S: Supervisor<HttpNewActor<NA>> + Clone + 'static,
    NA: NewActor<Argument = Connection> + Clone + 'static,
{
    let new_actor = HttpNewActor { new_actor };
    tcp::server::setup(address, supervisor, new_actor, options)
}

/// A intermediate structure that implements [`NewActor`], creating an actor
/// that spawn a new actor for each incoming HTTP connection.
///
/// See [`setup`] to create this and the [module documentation] for examples.
///
/// [module documentation]: crate::server
pub type Setup<S, NA> = tcp::server::Setup<S, HttpNewActor<NA>>;

/// Maps `NA` to accept `TcpStream` as argument, creating a [`Connection`].
#[derive(Debug, Clone)]
pub struct HttpNewActor<NA> {
    new_actor: NA,
}

impl<NA> NewActor for HttpNewActor<NA>
where
    NA: NewActor<Argument = Connection>,
{
    type Message = NA::Message;
    type Argument = TcpStream;
    type Actor = NA::Actor;
    type Error = NA::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        stream: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let conn = Connection::new(stream);
        self.new_actor.new(ctx, conn)
    }

    fn name() -> &'static str {
        NA::name()
    }
}

/// HTTP connection.
///
/// This wraps a TCP stream from which [HTTP requests] are read and [HTTP
/// responses] are send to.
///
/// It's advisable to set `TCP_NODELAY` using [`Connection::set_nodelay`] as the
/// `Connection` uses internally buffering, meaning only bodies with small
/// chunks would benefit from `TCP_NODELAY`.
///
/// [HTTP requests]: Request
/// [HTTP responses]: crate::Response
#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Number of bytes of `buf` that are already parsed.
    /// NOTE: this may be larger then `buf.len()`, in which case a `Body` was
    /// dropped without reading it entirely.
    parsed_bytes: usize,
    /// The HTTP version of the last request.
    last_version: Option<Version>,
    /// The HTTP method of the last request.
    last_method: Option<Method>,
}

impl Connection {
    /// Create a new `Connection`.
    fn new(stream: TcpStream) -> Connection {
        Connection {
            stream,
            buf: Vec::with_capacity(BUF_SIZE),
            parsed_bytes: 0,
            last_version: None,
            last_method: None,
        }
    }

    /// Parse the next request from the connection.
    ///
    /// # Notes
    ///
    /// Most [`RequestError`]s can't be recovered from and the connection should
    /// be closed when hitting them, see [`RequestError::should_close`]. If the
    /// connection is not closed and `next_request` is called again it will
    /// likely return the same error (but this is not guaranteed).
    ///
    /// Also see the [`Connection::last_request_version`] and
    /// [`Connection::last_request_method`] functions to properly respond to
    /// request errors.
    #[allow(clippy::too_many_lines)] // TODO.
    pub async fn next_request<'a>(&'a mut self) -> Result<Option<Request<Body<'a>>>, RequestError> {
        // NOTE: not resetting the version as that doesn't change between
        // requests.
        self.last_method = None;

        let mut too_short = 0;
        loop {
            // In case of pipelined requests it could be that while reading a
            // previous request's body it partially read the head of the next
            // (this) request. To handle this we first attempt to parse the
            // request if we have more than zero bytes (of the next request) in
            // the first iteration of the loop.
            while self.parsed_bytes >= self.buf.len() || self.buf.len() <= too_short {
                // While we didn't read the entire previous request body, or
                // while we have less than `too_short` bytes we try to receive
                // some more bytes.

                if self.recv().await? {
                    return if self.buf.is_empty() {
                        // Read the entire stream, so we're done.
                        Ok(None)
                    } else {
                        // Couldn't read any more bytes, but we still have bytes
                        // in the buffer. This means it contains a partial
                        // request.
                        Err(RequestError::IncompleteRequest)
                    };
                }
            }

            let mut headers = MaybeUninit::uninit_array::<MAX_HEADERS>();
            let mut request = httparse::Request::new(&mut []);
            // SAFETY: because we received until at least `self.parsed_bytes >=
            // self.buf.len()` above, we can safely slice the buffer..
            match request.parse_with_uninit_headers(&self.buf[self.parsed_bytes..], &mut headers) {
                Ok(httparse::Status::Complete(head_length)) => {
                    self.parsed_bytes += head_length;

                    // SAFETY: all these unwraps are safe because `parse` above
                    // ensures there all `Some`.
                    let Ok(method) = request.method.unwrap().parse() else {
                        return Err(RequestError::UnknownMethod);
                    };
                    self.last_method = Some(method);
                    let path = request.path.unwrap().to_string();
                    let version = map_version_byte(request.version.unwrap());
                    self.last_version = Some(version);

                    // RFC 7230 section 3.3.3 Message Body Length.
                    let mut body_length: Option<BodyLength> = None;
                    let headers =
                        Headers::from_httparse_headers(request.headers, |name, value| {
                            if *name == HeaderName::CONTENT_LENGTH {
                                // RFC 7230 section 3.3.3 point 4:
                                // > If a message is received without
                                // > Transfer-Encoding and with either multiple
                                // > Content-Length header fields having differing
                                // > field-values or a single Content-Length header
                                // > field having an invalid value, then the message
                                // > framing is invalid and the recipient MUST treat
                                // > it as an unrecoverable error. If this is a
                                // > request message, the server MUST respond with a
                                // > 400 (Bad Request) status code and then close
                                // > the connection.
                                if let Ok(length) = FromHeaderValue::from_bytes(value) {
                                    match body_length.as_mut() {
                                        Some(BodyLength::Known(body_length))
                                            if *body_length == length => {}
                                        Some(BodyLength::Known(_)) => {
                                            return Err(RequestError::DifferentContentLengths)
                                        }
                                        Some(BodyLength::Chunked) => {
                                            return Err(
                                                RequestError::ContentLengthAndTransferEncoding,
                                            )
                                        }
                                        // RFC 7230 section 3.3.3 point 5:
                                        // > If a valid Content-Length header field
                                        // > is present without Transfer-Encoding,
                                        // > its decimal value defines the expected
                                        // > message body length in octets.
                                        None => body_length = Some(BodyLength::Known(length)),
                                    }
                                } else {
                                    return Err(RequestError::InvalidContentLength);
                                }
                            } else if *name == HeaderName::TRANSFER_ENCODING {
                                let mut encodings = value.split(|b| *b == b',').peekable();
                                while let Some(encoding) = encodings.next() {
                                    match trim_ws(encoding) {
                                        b"chunked" => {
                                            // RFC 7230 section 3.3.3 point 3:
                                            // > If a Transfer-Encoding header field
                                            // > is present in a request and the
                                            // > chunked transfer coding is not the
                                            // > final encoding, the message body
                                            // > length cannot be determined
                                            // > reliably; the server MUST respond
                                            // > with the 400 (Bad Request) status
                                            // > code and then close the connection.
                                            if encodings.peek().is_some() {
                                                return Err(
                                                    RequestError::ChunkedNotLastTransferEncoding,
                                                );
                                            }

                                            // RFC 7230 section 3.3.3 point 3:
                                            // > If a message is received with both
                                            // > a Transfer-Encoding and a
                                            // > Content-Length header field, the
                                            // > Transfer-Encoding overrides the
                                            // > Content-Length. Such a message
                                            // > might indicate an attempt to
                                            // > perform request smuggling (Section
                                            // > 9.5) or response splitting (Section
                                            // > 9.4) and ought to be handled as an
                                            // > error.
                                            if body_length.is_some() {
                                                return Err(
                                                    RequestError::ContentLengthAndTransferEncoding,
                                                );
                                            }

                                            body_length = Some(BodyLength::Chunked);
                                        }
                                        b"identity" => {} // No changes.
                                        // TODO: support "compress", "deflate" and
                                        // "gzip".
                                        _ => return Err(RequestError::UnsupportedTransferEncoding),
                                    }
                                }
                            }
                            Ok(())
                        })?;

                    let kind = match body_length {
                        Some(BodyLength::Known(left)) => BodyKind::Oneshot { left },
                        Some(BodyLength::Chunked) => {
                            #[allow(clippy::cast_possible_truncation)] // For truncate below.
                            match httparse::parse_chunk_size(&self.buf[self.parsed_bytes..]) {
                                Ok(httparse::Status::Complete((idx, chunk_size))) => {
                                    self.parsed_bytes += idx;
                                    BodyKind::Chunked {
                                        // FIXME: add check here. It's fine on
                                        // 64 bit (only currently supported).
                                        left_in_chunk: chunk_size as usize,
                                        read_complete: chunk_size == 0,
                                    }
                                }
                                Ok(httparse::Status::Partial) => BodyKind::Chunked {
                                    left_in_chunk: 0,
                                    read_complete: false,
                                },
                                Err(_) => return Err(RequestError::InvalidChunkSize),
                            }
                        }
                        // RFC 7230 section 3.3.3 point 6:
                        // > If this is a request message and none of the above
                        // > are true, then the message body length is zero (no
                        // > message body is present).
                        None => BodyKind::Oneshot { left: 0 },
                    };
                    let body = Body { conn: self, kind };
                    return Ok(Some(Request::new(method, path, version, headers, body)));
                }
                Ok(httparse::Status::Partial) => {
                    // Buffer doesn't include the entire request head, try
                    // reading more bytes (in the next iteration).
                    too_short = self.buf.len();
                    self.last_method = request.method.and_then(|m| m.parse().ok());
                    if let Some(version) = request.version {
                        self.last_version = Some(map_version_byte(version));
                    }

                    if too_short >= MAX_HEAD_SIZE {
                        return Err(RequestError::HeadTooLarge);
                    }

                    continue;
                }
                Err(err) => return Err(RequestError::from_httparse(err)),
            }
        }
    }

    /// Returns the HTTP version of the last (partial) request.
    ///
    /// This can be used in cases where [`Connection::next_request`] returns a
    /// [`RequestError`].
    ///
    /// # Examples
    ///
    /// Responding to a [`RequestError`].
    ///
    /// ```
    /// use heph_http::{Response, Headers, StatusCode, Version, Method};
    /// use heph_http::server::{Connection, RequestError};
    /// use heph_http::body::OneshotBody;
    ///
    /// # return;
    /// # #[allow(unreachable_code)]
    /// # {
    /// let mut conn: Connection = /* From HttpServer. */
    /// # panic!("can't actually run example");
    ///
    /// // Reading a request returned this error.
    /// let err = RequestError::IncompleteRequest;
    ///
    /// // We can use `last_request_method` to determine the method of the last
    /// // request, which is used to determine if we need to send a body.
    /// let request_method = conn.last_request_method().unwrap_or(Method::Get);
    ///
    /// // We can use `last_request_version` to determine the client preferred
    /// // HTTP version, or default to the server's preferred version (HTTP/1.1
    /// // here).
    /// let version = conn.last_request_version().unwrap_or(Version::Http11);
    ///
    /// let msg = format!("Bad request: {err}");
    /// let body = OneshotBody::new(msg.as_bytes());
    ///
    /// // Respond with the response.
    /// conn.send_response(request_method, version, StatusCode::BAD_REQUEST, &Headers::EMPTY, body);
    ///
    /// // Close the connection if the error is fatal.
    /// if err.should_close() {
    ///     return;
    /// }
    /// # }
    /// ```
    pub fn last_request_version(&self) -> Option<Version> {
        self.last_version
    }

    /// Returns the HTTP method of the last (partial) request.
    ///
    /// This can be used in cases where [`Connection::next_request`] returns a
    /// [`RequestError`].
    ///
    /// # Examples
    ///
    /// See [`Connection::last_request_version`] for an example that responds to
    /// a [`RequestError`], which uses `last_request_method`.
    pub fn last_request_method(&self) -> Option<Method> {
        self.last_method
    }

    /// Respond to the last parsed request.
    ///
    /// # Notes
    ///
    /// This uses information from the last call to [`Connection::next_request`]
    /// to respond to the request correctly. For example it uses the HTTP
    /// [`Method`] to determine whether or not to send the body (as HEAD request
    /// don't expect a body). When reading multiple requests from the connection
    /// before responding use [`Connection::send_response`] directly.
    ///
    /// See the notes for [`Connection::send_response`], they apply to this
    /// function also.
    pub async fn respond<B>(
        &mut self,
        status: StatusCode,
        headers: &Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body,
    {
        let req_method = self.last_method.unwrap_or(Method::Get);
        let version = self.last_version.unwrap_or(Version::Http11).highest_minor();
        self.send_response(req_method, version, status, headers, body)
            .await
    }

    /// Respond to the last parsed request with `response`.
    ///
    /// See [`Connection::respond`] for more documentation.
    pub async fn respond_with<B>(&mut self, response: Response<B>) -> io::Result<()>
    where
        B: crate::Body,
    {
        let (head, body) = response.split();
        self.respond(head.status(), head.headers(), body).await
    }

    /// Send a [`Response`].
    ///
    /// Arguments:
    ///  * `request_method` is the method used by the [`Request`], used to
    ///    determine if a body needs to be send.
    ///  * `version`, `status`, `headers` and `body` make up the HTTP
    ///    [`Response`].
    ///
    /// In most cases it's easier to use [`Connection::respond`], only when
    /// reading two requests before responding is this function useful.
    ///
    /// [`Response`]: crate::Response
    ///
    /// # Notes
    ///
    /// This automatically sets the "Content-Length" or "Transfer-Encoding",
    /// "Connection" and "Date" headers if not provided in `headers`.
    ///
    /// If `request_method.`[`expects_body()`] or `status.`[`includes_body()`]
    /// returns `false` this will not write the body to the connection.
    ///
    /// [`expects_body()`]: Method::expects_body
    /// [`includes_body()`]: StatusCode::includes_body
    pub async fn send_response<B>(
        &mut self,
        request_method: Method,
        // Response data:
        version: Version,
        status: StatusCode,
        headers: &Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body,
    {
        let mut itoa_buf = itoa::Buffer::new();

        // Clear bytes from the previous request, keeping the bytes of any
        // unprocessed request(s).
        self.clear_buffer();

        // If the read buffer is empty we can use, otherwise we need to create a
        // new buffer to ensure we don't lose bytes.
        let mut http_head = if self.buf.is_empty() {
            take(&mut self.buf)
        } else {
            Vec::with_capacity(INIT_HEAD_SIZE)
        };

        // Format the status-line (RFC 7230 section 3.1.2).
        http_head.extend_from_slice(version.as_str().as_bytes());
        http_head.push(b' ');
        http_head.extend_from_slice(itoa_buf.format(status.0).as_bytes());
        // NOTE: we're not sending a reason-phrase, but the space is required
        // before \r\n.
        http_head.extend_from_slice(b" \r\n");

        // Format the headers (RFC 7230 section 3.2).
        let mut set_connection_header = false;
        let mut set_content_length_header = false;
        let mut set_transfer_encoding_header = false;
        let mut set_date_header = false;
        for header in headers {
            let name = header.name();
            // Field-name:
            http_head.extend_from_slice(name.as_ref().as_bytes());
            // NOTE: spacing after the colon (`:`) is optional.
            http_head.extend_from_slice(b": ");
            // Append the header's value.
            // NOTE: `header.value` shouldn't contain CRLF (`\r\n`).
            http_head.extend_from_slice(header.value());
            http_head.extend_from_slice(b"\r\n");

            if name == &HeaderName::CONNECTION {
                set_connection_header = true;
            } else if name == &HeaderName::CONTENT_LENGTH {
                set_content_length_header = true;
            } else if name == &HeaderName::TRANSFER_ENCODING {
                set_transfer_encoding_header = true;
            } else if name == &HeaderName::DATE {
                set_date_header = true;
            }
        }

        // Provide the "Connection" header if the user didn't.
        if !set_connection_header && matches!(version, Version::Http10) {
            // Per RFC 7230 section 6.3, HTTP/1.0 needs the "Connection:
            // keep-alive" header to persistent the connection. Connections
            // using HTTP/1.1 persistent by default.
            http_head.extend_from_slice(b"Connection: keep-alive\r\n");
        }

        // Provide the "Date" header if the user didn't.
        if !set_date_header {
            let now = HttpDate::from(SystemTime::now());
            write!(&mut http_head, "Date: {now}\r\n").unwrap();
        }

        // Provide the "Conent-Length" or "Transfer-Encoding" header if the user
        // didn't.
        let mut send_body = true;
        if !set_content_length_header && !set_transfer_encoding_header {
            match body.length() {
                _ if !request_method.expects_body() || !status.includes_body() => {
                    send_body = false;
                    extend_content_length_header(&mut http_head, &mut itoa_buf, 0);
                }
                BodyLength::Known(length) => {
                    extend_content_length_header(&mut http_head, &mut itoa_buf, length);
                }
                BodyLength::Chunked => {
                    http_head.extend_from_slice(b"Transfer-Encoding: chunked\r\n");
                }
            }
        }

        // End of the HTTP head.
        http_head.extend_from_slice(b"\r\n");

        // Write the response to the stream.
        let mut http_head = if send_body {
            body.write_message(&mut self.stream, http_head).await?
        } else {
            self.stream.send_all(http_head).await?
        };

        if self.buf.is_empty() {
            // We used the read buffer so let's put it back.
            http_head.clear();
            self.buf = http_head;
        }

        Ok(())
    }

    /// See [`TcpStream::peer_addr`].
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// See [`TcpStream::local_addr`].
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    /// See [`TcpStream::set_nodelay`].
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.stream.set_nodelay(nodelay)
    }

    /// See [`TcpStream::nodelay`].
    pub fn nodelay(&self) -> io::Result<bool> {
        self.stream.nodelay()
    }

    async fn read_chunk(
        &mut self,
        // Fields of `BodyKind::Chunked`:
        left_in_chunk: &mut usize,
        read_complete: &mut bool,
    ) -> Result<(), RequestError> {
        loop {
            match httparse::parse_chunk_size(&self.buf[self.parsed_bytes..]) {
                #[allow(clippy::cast_possible_truncation)] // For truncate below.
                Ok(httparse::Status::Complete((idx, chunk_size))) => {
                    self.parsed_bytes += idx;
                    if chunk_size == 0 {
                        *read_complete = true;
                    }
                    // FIXME: add check here. It's fine on 64 bit (only currently
                    // supported).
                    *left_in_chunk = chunk_size as usize;
                    return Ok(());
                }
                Ok(httparse::Status::Partial) => {} // Read some more data below.
                Err(_) => return Err(RequestError::InvalidChunkSize),
            }

            if self.recv().await? {
                return Err(RequestError::IncompleteRequest);
            }
        }
    }

    /// Returns true if we read all bytes (i.e. we read 0 bytes).
    async fn recv(&mut self) -> io::Result<bool> {
        // Ensure we have space in the buffer to read into.
        self.clear_buffer();
        self.buf.reserve(MIN_READ_SIZE);

        let buf_len = self.buf.len();
        self.buf = self.stream.recv(take(&mut self.buf)).await?;
        Ok(self.buf.len() == buf_len)
    }

    /// Clear parsed request(s) from the buffer.
    fn clear_buffer(&mut self) {
        let buf_len = self.buf.len();
        if self.parsed_bytes >= buf_len {
            // Parsed all bytes in the buffer, so we can clear it.
            self.buf.clear();
            self.parsed_bytes = 0;
        }

        // TODO: move bytes to the start.
    }
}

/// Add "Content-Length" header to `buf`.
fn extend_content_length_header(
    buf: &mut Vec<u8>,
    itoa_buf: &mut itoa::Buffer,
    content_length: usize,
) {
    buf.extend_from_slice(b"Content-Length: ");
    buf.extend_from_slice(itoa_buf.format(content_length).as_bytes());
    buf.extend_from_slice(b"\r\n");
}

/// Body of HTTP [`Request`] read from a [`Connection`].
///
/// # Notes
///
/// If the body is not (completely) read before this is dropped it will still
/// removed from the `Connection`.
#[derive(Debug)]
pub struct Body<'a> {
    conn: &'a mut Connection,
    kind: BodyKind,
}

#[derive(Debug)]
enum BodyKind {
    /// No encoding.
    Oneshot {
        /// Number of unread (by the user) bytes.
        left: usize,
    },
    /// Chunked transfer encoding.
    Chunked {
        /// Number of unread (by the user) bytes in this chunk.
        left_in_chunk: usize,
        /// Read all chunks.
        read_complete: bool,
    },
}

impl<'a> Body<'a> {
    /// Returns the length of the body (in bytes) *left*.
    ///
    /// Calling this before [`recv`] or [`recv_vectored`] will return the
    /// original body length, after removing bytes from the body this will
    /// return the *remaining* length.
    ///
    /// The body length is determined by the "Content-Length" or
    /// "Transfer-Encoding" header, or 0 if neither are present.
    ///
    /// [`recv`]: Body::recv
    /// [`recv_vectored`]: Body::recv_vectored
    pub fn len(&self) -> BodyLength {
        match self.kind {
            BodyKind::Oneshot { left } => BodyLength::Known(left),
            BodyKind::Chunked { .. } => BodyLength::Chunked,
        }
    }

    /// Returns `true` if the body is completely read (or was empty to begin
    /// with).
    ///
    /// # Notes
    ///
    /// This can return `false` for empty bodies using chunked encoding if not
    /// enough bytes have been read yet. Using chunked encoding we don't know
    /// the length upfront as it it's determined by reading the length of each
    /// chunk. If the send request only contained the HTTP head (i.e. no body)
    /// and uses chunked encoding this would return `false`, as body length is
    /// unknown and thus not empty. However if the body would then send a single
    /// empty chunk (signaling the end of the body), this would return `true` as
    /// it turns out the body is indeed empty.
    pub fn is_empty(&self) -> bool {
        match self.kind {
            BodyKind::Oneshot { left } => left == 0,
            BodyKind::Chunked {
                left_in_chunk,
                read_complete,
            } => read_complete && left_in_chunk == 0,
        }
    }

    /// Returns `true` if the body is chunked.
    pub fn is_chunked(&self) -> bool {
        matches!(self.kind, BodyKind::Chunked { .. })
    }

    /// Receive bytes from the request body, writing them into `buf`.
    pub async fn recv<B: BufMut>(&mut self, mut buf: B) -> io::Result<B> {
        loop {
            // Quick return for if we read all bytes in the body already.
            if self.is_empty() {
                return Ok(buf);
            }

            // First try to copy already buffered bytes.
            let buf_bytes = self.buf_bytes();
            if !buf_bytes.is_empty() {
                let written = buf.extend_from_slice(buf_bytes);
                self.processed(written);
                return Ok(buf);
            }

            // We need to ensure that we don't read another response head or
            // chunk head into `buf`. So we need to determine a limit on the
            // amount of bytes we can safely read. We only can't determine that
            // for the case were we read an entire chunk, but don't know
            // anything about the next chunk. In this case we need our own
            // buffer to ensure we don't lose not-body bytes to the user's
            // `buf`fer.
            let limit = match &mut self.kind {
                BodyKind::Oneshot { left } => *left,
                BodyKind::Chunked {
                    left_in_chunk,
                    read_complete,
                } => {
                    if *left_in_chunk == 0 {
                        self.conn.read_chunk(left_in_chunk, read_complete).await?;
                        // Read from the client's buffer again.
                        continue;
                    }
                    *left_in_chunk
                }
            };

            let len_before = buf.spare_capacity();
            let limited_buf = self.conn.stream.recv(buf.limit(limit)).await?;
            let buf = limited_buf.into_inner();
            self.processed(buf.spare_capacity() - len_before);
            return Ok(buf);
        }
    }

    /// Receive bytes from the request body, writing them into `bufs` using
    /// vectored I/O.
    pub async fn recv_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        mut bufs: B,
    ) -> io::Result<B> {
        loop {
            // Quick return for if we read all bytes in the body already.
            if self.is_empty() {
                return Ok(bufs);
            }

            // First try to copy already buffered bytes.
            let buf_bytes = self.buf_bytes();
            if !buf_bytes.is_empty() {
                let written = bufs.extend_from_slice(buf_bytes);
                self.processed(written);
                return Ok(bufs);
            }

            // We need to ensure that we don't read another response head or
            // chunk head into `buf`. So we need to determine a limit on the
            // amount of bytes we can safely read. We only can't determine that
            // for the case were we read an entire chunk, but don't know
            // anything about the next chunk. In this case we need our own
            // buffer to ensure we don't lose not-body bytes to the user's
            // `buf`fer.
            let limit = match &mut self.kind {
                BodyKind::Oneshot { left } => *left,
                BodyKind::Chunked {
                    left_in_chunk,
                    read_complete,
                } => {
                    if *left_in_chunk == 0 {
                        self.conn.read_chunk(left_in_chunk, read_complete).await?;
                        // Read from the client's buffer again.
                        continue;
                    }
                    *left_in_chunk
                }
            };

            let len_before = bufs.total_spare_capacity();
            let limited_bufs = self.conn.stream.recv_vectored(bufs.limit(limit)).await?;
            let bufs = limited_bufs.into_inner();
            self.processed(bufs.total_spare_capacity() - len_before);
            return Ok(bufs);
        }
    }

    /// Returns the bytes currently in the buffer.
    ///
    /// This is limited to the bytes of this request/chunk, i.e. it doesn't
    /// contain the next request/chunk.
    fn buf_bytes(&self) -> &[u8] {
        let bytes = &self.conn.buf[self.conn.parsed_bytes..];
        let left = match self.kind {
            BodyKind::Oneshot { left } => left,
            BodyKind::Chunked { left_in_chunk, .. } => left_in_chunk,
        };
        if bytes.len() > left {
            &bytes[..left]
        } else {
            bytes
        }
    }

    /// Mark `n` bytes are processed.
    fn processed(&mut self, n: usize) {
        // TODO: should this be `unsafe`? We don't do underflow checks...
        match &mut self.kind {
            BodyKind::Oneshot { left } => *left -= n,
            BodyKind::Chunked { left_in_chunk, .. } => *left_in_chunk -= n,
        }
        self.conn.parsed_bytes += n;
    }
}

impl<'a> Drop for Body<'a> {
    fn drop(&mut self) {
        if self.is_empty() {
            // Empty body, then we're done quickly.
            return;
        }

        // Mark the entire body as parsed.
        // NOTE: `Connection` handles the case where we didn't read the entire
        // body yet.
        match self.kind {
            BodyKind::Oneshot { left } => self.conn.parsed_bytes += left,
            BodyKind::Chunked {
                left_in_chunk,
                read_complete,
            } => {
                if read_complete {
                    // Read all chunks.
                    debug_assert_eq!(left_in_chunk, 0);
                } else {
                    // FIXME: don't panic here.
                    todo!("remove chunked body from connection");
                }
            }
        }
    }
}

/// Error parsing HTTP request.
#[non_exhaustive]
#[derive(Debug)]
pub enum RequestError {
    /// Missing part of request.
    IncompleteRequest,
    /// HTTP Head (start line and headers) is too large.
    ///
    /// Limit is defined by [`MAX_HEAD_SIZE`].
    HeadTooLarge,
    /// Value in the "Content-Length" header is invalid.
    InvalidContentLength,
    /// Multiple "Content-Length" headers were present with differing values.
    DifferentContentLengths,
    /// Invalid byte in header name.
    InvalidHeaderName,
    /// Invalid byte in header value.
    InvalidHeaderValue,
    /// Number of headers send in the request is larger than [`MAX_HEADERS`].
    TooManyHeaders,
    /// Unsupported "Transfer-Encoding" header.
    UnsupportedTransferEncoding,
    /// Request has a "Transfer-Encoding" header with a chunked encoding, but
    /// it's not the final encoding, then the message body length cannot be
    /// determined reliably.
    ///
    /// See RFC 7230 section 3.3.3 point 3.
    ChunkedNotLastTransferEncoding,
    /// Request contains both "Content-Length" and "Transfer-Encoding" headers.
    ///
    /// An attacker might attempt to "smuggle a request" ("HTTP Request
    /// Smuggling", Linhart et al., June 2005) or "split a response" ("Divide
    /// and Conquer - HTTP Response Splitting, Web Cache Poisoning Attacks, and
    /// Related Topics", Klein, March 2004). RFC 7230 (see section 3.3.3 point
    /// 3) says that this "ought to be handled as an error", and so we do.
    ContentLengthAndTransferEncoding,
    /// Invalid byte where token is required.
    InvalidToken,
    /// Invalid byte in new line.
    InvalidNewLine,
    /// Invalid byte in HTTP version.
    InvalidVersion,
    /// Unknown HTTP method, not in [`Method`].
    UnknownMethod,
    /// Chunk size is invalid.
    InvalidChunkSize,
    /// I/O error.
    Io(io::Error),
}

impl RequestError {
    /// Returns the proper status code for a given error.
    pub const fn proper_status_code(&self) -> StatusCode {
        use RequestError::*;
        // See the parsing code for various references to the RFC(s) that
        // determine the values here.
        match self {
            IncompleteRequest
            | HeadTooLarge
            | InvalidContentLength
            | DifferentContentLengths
            | InvalidHeaderName
            | InvalidHeaderValue
            | TooManyHeaders
            | ChunkedNotLastTransferEncoding
            | ContentLengthAndTransferEncoding
            | InvalidToken
            | InvalidNewLine
            | InvalidVersion
            | InvalidChunkSize => StatusCode::BAD_REQUEST,
            // RFC 7230 section 3.3.1:
            // > A server that receives a request message with a transfer coding
            // > it does not understand SHOULD respond with 501 (Not
            // > Implemented).
            UnsupportedTransferEncoding
            // RFC 7231 section 4.1:
            // > When a request method is received that is unrecognized or not
            // > implemented by an origin server, the origin server SHOULD
            // > respond with the 501 (Not Implemented) status code.
            | UnknownMethod => StatusCode::NOT_IMPLEMENTED,
            Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns `true` if the connection should be closed based on the error
    /// (after sending a error response).
    pub const fn should_close(&self) -> bool {
        use RequestError::*;
        // See the parsing code for various references to the RFC(s) that
        // determine the values here.
        match self {
            IncompleteRequest
            | HeadTooLarge
            | InvalidContentLength
            | DifferentContentLengths
            | InvalidHeaderName
            | InvalidHeaderValue
            | UnsupportedTransferEncoding
            | TooManyHeaders
            | ChunkedNotLastTransferEncoding
            | ContentLengthAndTransferEncoding
            | InvalidToken
            | InvalidNewLine
            | InvalidVersion
            | InvalidChunkSize
            | Io(_) => true,
            UnknownMethod => false,
        }
    }

    /// Returns a response with the [proper status code] set and possibly the
    /// [Connection] header if the [connection should be closed].
    ///
    /// [proper status code]: RequestError::proper_status_code
    /// [Connection]: HeaderName::CONNECTION
    /// [connection should be closed]: RequestError::should_close
    pub fn response(&self) -> Response<EmptyBody> {
        let mut response = Response::build_new(self.proper_status_code());
        if self.should_close() {
            response
                .headers_mut()
                .append(Header::new(HeaderName::CONNECTION, b"close"));
        }
        response
    }

    fn from_httparse(err: httparse::Error) -> RequestError {
        use httparse::Error::*;
        match err {
            HeaderName => RequestError::InvalidHeaderName,
            HeaderValue => RequestError::InvalidHeaderValue,
            Token => RequestError::InvalidToken,
            NewLine => RequestError::InvalidNewLine,
            Version => RequestError::InvalidVersion,
            TooManyHeaders => RequestError::TooManyHeaders,
            // Requests never contain a status, only responses do, but we don't
            // want a panic branch (from `unreachable!`) here.
            Status => RequestError::IncompleteRequest,
        }
    }

    /// Returns a static error message for the error.
    #[rustfmt::skip]
    pub fn as_str(&self) -> &'static str {
        use RequestError::*;
        match self {
            IncompleteRequest => "incomplete request",
            HeadTooLarge => "head too large",
            InvalidContentLength => "invalid Content-Length header",
            DifferentContentLengths => "different Content-Length headers",
            InvalidHeaderName => "invalid header name",
            InvalidHeaderValue => "invalid header value",
            TooManyHeaders => "too many header",
            UnsupportedTransferEncoding => "unsupported Transfer-Encoding",
            ChunkedNotLastTransferEncoding => "invalid Transfer-Encoding header",
            ContentLengthAndTransferEncoding => {
                "provided both Content-Length and Transfer-Encoding headers"
            }
            InvalidToken | InvalidNewLine => "invalid request syntax",
            InvalidVersion => "invalid version",
            UnknownMethod => "unknown method",
            InvalidChunkSize => "invalid chunk size",
            Io(_) => "I/O error",
        }
    }
}

impl From<io::Error> for RequestError {
    fn from(err: io::Error) -> RequestError {
        if let io::ErrorKind::UnexpectedEof = err.kind() {
            RequestError::IncompleteRequest
        } else {
            RequestError::Io(err)
        }
    }
}

impl From<RequestError> for io::Error {
    fn from(err: RequestError) -> io::Error {
        match err {
            RequestError::Io(err) => err,
            err => io::Error::new(io::ErrorKind::InvalidData, err.as_str()),
        }
    }
}

impl From<DeadlinePassed> for RequestError {
    fn from(_: DeadlinePassed) -> RequestError {
        RequestError::Io(io::ErrorKind::TimedOut.into())
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Io(err) => err.fmt(f),
            err => err.as_str().fmt(f),
        }
    }
}

/// The message type used by the HTTP server.
///
#[doc(inline)]
pub use heph_rt::net::tcp::server::Message;

/// Error returned by the HTTP server.
///
#[doc(inline)]
pub use heph_rt::net::tcp::server::Error;
