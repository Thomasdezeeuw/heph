// TODO: `S: Supervisor` currently uses `TcpStream` as argument due to `ArgMap`.
//       Maybe disconnect `S` from `NA`?
//
// TODO: Continue reading RFC 7230 section 4 Transfer Codings.
//
// TODO: RFC 7230 section 3.3.3 point 5:
// > If the sender closes the connection or the recipient
// > times out before the indicated number of octets are
// > received, the recipient MUST consider the message to be
// > incomplete and close the connection.

//! Module with the HTTP server implementation.

use std::cmp::min;
use std::fmt;
use std::future::Future;
use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::ready;
use std::task::{self, Poll};
use std::time::SystemTime;

use heph::{actor, Actor, NewActor, Supervisor};
use heph_rt::bytes::{Bytes, BytesVectored};
use heph_rt::net::{tcp, TcpServer, TcpStream};
use heph_rt::spawn::{ActorOptions, Spawn};
use httpdate::HttpDate;

use crate::body::{BodyLength, EmptyBody};
use crate::head::header::{FromHeaderValue, Header, HeaderName, Headers};
use crate::{
    map_version_byte, trim_ws, Method, Request, Response, StatusCode, Version, BUF_SIZE,
    MAX_HEADERS, MAX_HEAD_SIZE, MIN_READ_SIZE,
};

/// A intermediate structure that implements [`NewActor`], creating
/// [`HttpServer`].
///
/// See [`HttpServer::setup`] to create this and [`HttpServer`] for examples.
#[derive(Debug)]
pub struct Setup<S, NA> {
    inner: tcp::server::Setup<S, ArgMap<NA>>,
}

impl<S, NA> Setup<S, NA> {
    /// Returns the address the server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }
}

impl<S, NA> NewActor for Setup<S, NA>
where
    S: Supervisor<ArgMap<NA>> + Clone + 'static,
    NA: NewActor<Argument = (Connection, SocketAddr)> + Clone + 'static,
    NA::RuntimeAccess: heph_rt::Access + Spawn<S, ArgMap<NA>, NA::RuntimeAccess>,
{
    type Message = Message;
    type Argument = ();
    type Actor = HttpServer<S, NA>;
    type Error = io::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        self.inner.new(ctx, arg).map(|inner| HttpServer { inner })
    }
}

impl<S, NA> Clone for Setup<S, NA> {
    fn clone(&self) -> Setup<S, NA> {
        Setup {
            inner: self.inner.clone(),
        }
    }
}

/// An actor that starts a new actor for each accepted HTTP [`Connection`].
///
/// `HttpServer` has the same design as [`TcpServer`]. It accept `TcpStream`s
/// and converts those into HTTP [`Connection`]s, from which HTTP [`Request`]s
/// can be read and HTTP [`Response`]s can be written.
///
/// Similar to `TcpServer` this type works with thread-safe and thread-local
/// actors.
///
/// [`Response`]: crate::Response
///
/// # Graceful shutdown
///
/// Graceful shutdown is done by sending it a [`Terminate`] message. The HTTP
/// server can also handle (shutdown) process signals, see below for an example.
///
/// [`Terminate`]: heph::messages::Terminate
///
/// # Examples
///
/// ```rust
/// # #![feature(never_type)]
/// use std::borrow::Cow;
/// use std::io;
/// use std::net::SocketAddr;
/// use std::time::Duration;
///
/// use heph::actor::{self, Actor, NewActor};
/// use heph::supervisor::{Supervisor, SupervisorStrategy};
/// use heph::timer::Deadline;
/// use heph_http::body::OneshotBody;
/// use heph_http::{self as http, Header, HeaderName, Headers, HttpServer, Method, StatusCode};
/// use heph_rt::net::TcpStream;
/// use heph_rt::{Runtime, ThreadLocal};
/// use heph_rt::spawn::options::{ActorOptions, Priority};
/// use log::error;
///
/// fn main() -> Result<(), rt::Error> {
///     // Setup the HTTP server.
///     let actor = http_actor as fn(_, _, _) -> _;
///     let address = "127.0.0.1:7890".parse().unwrap();
///     let server = HttpServer::setup(address, conn_supervisor, actor, ActorOptions::default())
///         .map_err(rt::Error::setup)?;
///
///     // Build the runtime.
///     let mut runtime = Runtime::setup().use_all_cores().build()?;
///     // On each worker thread start our HTTP server.
///     runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
///         let options = ActorOptions::default().with_priority(Priority::LOW);
///         let server_ref = runtime_ref.try_spawn_local(ServerSupervisor, server, (), options)?;
///
/// #       server_ref.try_send(heph::messages::Terminate).unwrap();
///
///         // Allow graceful shutdown by responding to process signals.
///         runtime_ref.receive_signals(server_ref.try_map());
///         Ok(())
///     })?;
///     runtime.start()
/// }
///
/// /// Our supervisor for the TCP server.
/// #[derive(Copy, Clone, Debug)]
/// struct ServerSupervisor;
///
/// impl<NA> Supervisor<NA> for ServerSupervisor
/// where
///     NA: NewActor<Argument = (), Error = io::Error>,
///     NA::Actor: Actor<Error = http::server::Error<!>>,
/// {
///     fn decide(&mut self, err: http::server::Error<!>) -> SupervisorStrategy<()> {
///         use http::server::Error::*;
///         match err {
///             Accept(err) => {
///                 error!("error accepting new connection: {err}");
///                 SupervisorStrategy::Restart(())
///             }
///             NewActor(_) => unreachable!(),
///         }
///     }
///
///     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
///         error!("error restarting the TCP server: {err}");
///         SupervisorStrategy::Stop
///     }
///
///     fn second_restart_error(&mut self, err: io::Error) {
///         error!("error restarting the actor a second time: {err}");
///     }
/// }
///
/// fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
///     error!("error handling connection: {err}");
///     SupervisorStrategy::Stop
/// }
///
/// /// Our actor that handles a single HTTP connection.
/// async fn http_actor(
///     mut ctx: actor::Context<!, ThreadLocal>,
///     mut connection: http::Connection,
///     address: SocketAddr,
/// ) -> io::Result<()> {
///     // Set `TCP_NODELAY` on the `TcpStream`.
///     connection.set_nodelay(true)?;
///
///     let mut headers = Headers::EMPTY;
///     loop {
///         // Read the next request.
///         let (code, body, should_close) = match connection.next_request().await? {
///             Ok(Some(request)) => {
///                 // Only support GET/HEAD to "/", with an empty body.
///                 if request.path() != "/" {
///                     (StatusCode::NOT_FOUND, "Not found".into(), false)
///                 } else if !matches!(request.method(), Method::Get | Method::Head) {
///                     // Add the "Allow" header to show the HTTP methods we do
///                     // support.
///                     headers.append(Header::new(HeaderName::ALLOW, b"GET, HEAD"));
///                     let body = "Method not allowed".into();
///                     (StatusCode::METHOD_NOT_ALLOWED, body, false)
///                 } else if !request.body().is_empty() {
///                     (StatusCode::PAYLOAD_TOO_LARGE, "Not expecting a body".into(), true)
///                 } else {
///                     // Use the IP address as body.
///                     let body = Cow::from(address.ip().to_string());
///                     (StatusCode::OK, body, false)
///                 }
///             }
///             // No more requests.
///             Ok(None) => return Ok(()),
///             // Error parsing request.
///             Err(err) => {
///                 // Determine the correct status code to return.
///                 let code = err.proper_status_code();
///                 // Create a useful error message as body.
///                 let body = Cow::from(format!("Bad request: {err}"));
///                 (code, body, err.should_close())
///             }
///         };
///
///         // If we want to close the connection add the "Connection: close"
///         // header.
///         if should_close {
///             headers.append(Header::new(HeaderName::CONNECTION, b"close"));
///         }
///
///         // Send the body as a single payload.
///         let body = OneshotBody::new(body.as_bytes());
///         // Respond to the request.
///         connection.respond(code, &headers, body).await?;
///
///         if should_close {
///             return Ok(());
///         }
///         headers.clear();
///     }
/// }
/// ```
pub struct HttpServer<S, NA: NewActor<Argument = (Connection, SocketAddr)>> {
    inner: TcpServer<S, ArgMap<NA>>,
}

impl<S, NA> HttpServer<S, NA>
where
    S: Supervisor<ArgMap<NA>> + Clone + 'static,
    NA: NewActor<Argument = (Connection, SocketAddr)> + Clone + 'static,
{
    /// Create a new [server setup].
    ///
    /// Arguments:
    /// * `address`: the address to listen on.
    /// * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    /// * `new_actor`: the [`NewActor`] implementation to start each actor,
    ///   and
    /// * `options`: the actor options used to spawn the new actors.
    ///
    /// [server setup]: Setup
    pub fn setup(
        address: SocketAddr,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<Setup<S, NA>> {
        let new_actor = ArgMap { new_actor };
        TcpServer::setup(address, supervisor, new_actor, options).map(|inner| Setup { inner })
    }
}

impl<S, NA> Actor for HttpServer<S, NA>
where
    S: Supervisor<ArgMap<NA>> + Clone + 'static,
    NA: NewActor<Argument = (Connection, SocketAddr)> + Clone + 'static,
    NA::RuntimeAccess: heph_rt::Access + Spawn<S, ArgMap<NA>, NA::RuntimeAccess>,
{
    type Error = Error<NA::Error>;

    fn try_poll(
        self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let this = unsafe { self.map_unchecked_mut(|s| &mut s.inner) };
        this.try_poll(ctx)
    }
}

impl<S, NA> fmt::Debug for HttpServer<S, NA>
where
    S: fmt::Debug,
    NA: NewActor<Argument = (Connection, SocketAddr)> + fmt::Debug,
    NA::RuntimeAccess: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpServer")
            .field("inner", &self.inner)
            .finish()
    }
}

// TODO: better name. Like `TcpStreamToConnection`?
/// Maps `NA` to accept `(TcpStream, SocketAddr)` as argument, creating a
/// [`Connection`].
#[derive(Debug, Clone)]
pub struct ArgMap<NA> {
    new_actor: NA,
}

impl<NA> NewActor for ArgMap<NA>
where
    NA: NewActor<Argument = (Connection, SocketAddr)>,
{
    type Message = NA::Message;
    type Argument = (TcpStream, SocketAddr);
    type Actor = NA::Actor;
    type Error = NA::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        (stream, address): Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let conn = Connection::new(stream);
        self.new_actor.new(ctx, (conn, address))
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
    /// The return is a bit complex so let's break it down. The outer type is an
    /// [`io::Result`], which often needs to be handled seperately from errors
    /// in the request, e.g. by using `?`.
    ///
    /// Next is a `Result<Option<`[`Request`]`>, `[`RequestError`]`>`.
    /// `Ok(None)` is returned if the connection contains no more requests, i.e.
    /// when all bytes are read. If the connection contains a request it will
    /// return `Ok(Some(`[`Request`]`)`. If the request is somehow invalid it
    /// will return an `Err(`[`RequestError`]`)`.
    ///
    /// # Notes
    ///
    /// Most [`RequestError`]s can't be receover from and the connection should
    /// be closed when hitting them, see [`RequestError::should_close`]. If the
    /// connection is not closed and `next_request` is called again it will
    /// likely return the same error (but this is not guaranteed).
    ///
    /// Also see the [`Connection::last_request_version`] and
    /// [`Connection::last_request_method`] functions to properly respond to
    /// request errors.
    #[allow(clippy::too_many_lines)] // TODO.
    pub async fn next_request<'a>(
        &'a mut self,
    ) -> io::Result<Result<Option<Request<Body<'a>>>, RequestError>> {
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

                self.clear_buffer();
                self.buf.reserve(MIN_READ_SIZE);
                if self.stream.recv(&mut self.buf).await? == 0 {
                    return if self.buf.is_empty() {
                        // Read the entire stream, so we're done.
                        Ok(Ok(None))
                    } else {
                        // Couldn't read any more bytes, but we still have bytes
                        // in the buffer. This means it contains a partial
                        // request.
                        Ok(Err(RequestError::IncompleteRequest))
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
                    let method = match request.method.unwrap().parse() {
                        Ok(method) => method,
                        Err(_) => return Ok(Err(RequestError::UnknownMethod)),
                    };
                    self.last_method = Some(method);
                    let path = request.path.unwrap().to_string();
                    let version = map_version_byte(request.version.unwrap());
                    self.last_version = Some(version);

                    // RFC 7230 section 3.3.3 Message Body Length.
                    let mut body_length: Option<BodyLength> = None;
                    let res = Headers::from_httparse_headers(request.headers, |name, value| {
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
                                        return Err(RequestError::ContentLengthAndTransferEncoding)
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
                    });
                    let headers = match res {
                        Ok(headers) => headers,
                        Err(err) => return Ok(Err(err)),
                    };

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
                                Err(_) => return Ok(Err(RequestError::InvalidChunkSize)),
                            }
                        }
                        // RFC 7230 section 3.3.3 point 6:
                        // > If this is a request message and none of the above
                        // > are true, then the message body length is zero (no
                        // > message body is present).
                        None => BodyKind::Oneshot { left: 0 },
                    };
                    let body = Body { conn: self, kind };
                    return Ok(Ok(Some(Request::new(method, path, version, headers, body))));
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
                        return Ok(Err(RequestError::HeadTooLarge));
                    }

                    continue;
                }
                Err(err) => return Ok(Err(RequestError::from_httparse(err))),
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
    pub async fn respond<'b, B>(
        &mut self,
        status: StatusCode,
        headers: &Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body<'b> + 'b,
    {
        let req_method = self.last_method.unwrap_or(Method::Get);
        let version = self.last_version.unwrap_or(Version::Http11).highest_minor();
        self.send_response(req_method, version, status, headers, body)
            .await
    }

    /// Respond to the last parsed request with `response`.
    ///
    /// See [`Connection::respond`] for more documentation.
    pub async fn respond_with<'b, B>(&mut self, response: Response<B>) -> io::Result<()>
    where
        B: crate::Body<'b> + 'b,
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
    pub async fn send_response<'b, B>(
        &mut self,
        request_method: Method,
        // Response data:
        version: Version,
        status: StatusCode,
        headers: &Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body<'b> + 'b,
    {
        let mut itoa_buf = itoa::Buffer::new();

        // Clear bytes from the previous request, keeping the bytes of the
        // request.
        self.clear_buffer();
        let ignore_end = self.buf.len();

        // Format the status-line (RFC 7230 section 3.1.2).
        self.buf.extend_from_slice(version.as_str().as_bytes());
        self.buf.push(b' ');
        self.buf
            .extend_from_slice(itoa_buf.format(status.0).as_bytes());
        // NOTE: we're not sending a reason-phrase, but the space is required
        // before \r\n.
        self.buf.extend_from_slice(b" \r\n");

        // Format the headers (RFC 7230 section 3.2).
        let mut set_connection_header = false;
        let mut set_content_length_header = false;
        let mut set_transfer_encoding_header = false;
        let mut set_date_header = false;
        for header in headers.iter() {
            let name = header.name();
            // Field-name:
            self.buf.extend_from_slice(name.as_ref().as_bytes());
            // NOTE: spacing after the colon (`:`) is optional.
            self.buf.extend_from_slice(b": ");
            // Append the header's value.
            // NOTE: `header.value` shouldn't contain CRLF (`\r\n`).
            self.buf.extend_from_slice(header.value());
            self.buf.extend_from_slice(b"\r\n");

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
            self.buf.extend_from_slice(b"Connection: keep-alive\r\n");
        }

        // Provide the "Date" header if the user didn't.
        if !set_date_header {
            let now = HttpDate::from(SystemTime::now());
            write!(&mut self.buf, "Date: {now}\r\n").unwrap();
        }

        // Provide the "Conent-Length" or "Transfer-Encoding" header if the user
        // didn't.
        let mut send_body = true;
        if !set_content_length_header && !set_transfer_encoding_header {
            match body.length() {
                _ if !request_method.expects_body() || !status.includes_body() => {
                    send_body = false;
                    extend_content_length_header(&mut self.buf, &mut itoa_buf, 0)
                }
                BodyLength::Known(length) => {
                    extend_content_length_header(&mut self.buf, &mut itoa_buf, length)
                }
                BodyLength::Chunked => {
                    self.buf
                        .extend_from_slice(b"Transfer-Encoding: chunked\r\n");
                }
            }
        }

        // End of the HTTP head.
        self.buf.extend_from_slice(b"\r\n");

        // Write the response to the stream.
        let http_head = &self.buf[ignore_end..];
        if send_body {
            body.write_message(&mut self.stream, http_head).await?;
        } else {
            self.stream.send_all(http_head).await?;
        }

        // Remove the response head from the buffer.
        self.buf.truncate(ignore_end);
        Ok(())
    }

    /// See [`TcpStream::peer_addr`].
    pub fn peer_addr(&mut self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// See [`TcpStream::local_addr`].
    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    /// See [`TcpStream::set_ttl`].
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.stream.set_ttl(ttl)
    }

    /// See [`TcpStream::ttl`].
    pub fn ttl(&mut self) -> io::Result<u32> {
        self.stream.ttl()
    }

    /// See [`TcpStream::set_nodelay`].
    pub fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.stream.set_nodelay(nodelay)
    }

    /// See [`TcpStream::nodelay`].
    pub fn nodelay(&mut self) -> io::Result<bool> {
        self.stream.nodelay()
    }

    /// See [`TcpStream::keepalive`].
    pub fn keepalive(&self) -> io::Result<bool> {
        self.stream.keepalive()
    }

    /// See [`TcpStream::set_keepalive`].
    pub fn set_keepalive(&self, enable: bool) -> io::Result<()> {
        self.stream.set_keepalive(enable)
    }

    /// Clear parsed request(s) from the buffer.
    fn clear_buffer(&mut self) {
        let buf_len = self.buf.len();
        if self.parsed_bytes >= buf_len {
            // Parsed all bytes in the buffer, so we can clear it.
            self.buf.clear();
            self.parsed_bytes -= buf_len;
        }

        // TODO: move bytes to the start.
    }

    /// Recv bytes from the underlying stream, reading into `self.buf`.
    ///
    /// Returns an `UnexpectedEof` error if zero bytes are received.
    fn try_recv(&mut self) -> Poll<io::Result<usize>> {
        // Ensure we have space in the buffer to read into.
        self.clear_buffer();
        self.buf.reserve(MIN_READ_SIZE);

        loop {
            match self.stream.try_recv(&mut self.buf) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    }

    /// Read a HTTP body chunk.
    ///
    /// Returns an I/O error, or an `InvalidData` error if the chunk size is
    /// invalid.
    fn try_read_chunk(
        &mut self,
        // Fields of `BodyKind::Chunked`:
        left_in_chunk: &mut usize,
        read_complete: &mut bool,
    ) -> Poll<io::Result<()>> {
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
                    return Poll::Ready(Ok(()));
                }
                Ok(httparse::Status::Partial) => {} // Read some more data below.
                Err(_) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid chunk size",
                    )))
                }
            }

            let _ = ready!(self.try_recv())?;
        }
    }

    async fn read_chunk(
        &mut self,
        // Fields of `BodyKind::Chunked`:
        left_in_chunk: &mut usize,
        read_complete: &mut bool,
    ) -> io::Result<()> {
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
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid chunk size",
                    ))
                }
            }

            // Ensure we have space in the buffer to read into.
            self.clear_buffer();
            self.buf.reserve(MIN_READ_SIZE);

            if self.stream.recv(&mut self.buf).await? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
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
    /// Returns the length of the body (in bytes) *left*, or a
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

    /// Return the length of this chunk *left*, or the entire body in case of a
    /// oneshot body.
    fn chunk_len(&self) -> usize {
        match self.kind {
            BodyKind::Oneshot { left } => left,
            BodyKind::Chunked { left_in_chunk, .. } => left_in_chunk,
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
    pub const fn recv<B>(&'a mut self, buf: B) -> Recv<'a, B>
    where
        B: Bytes,
    {
        Recv { body: self, buf }
    }

    /// Receive bytes from the request body, writing them into `bufs`.
    pub const fn recv_vectored<B>(&'a mut self, bufs: B) -> RecvVectored<'a, B>
    where
        B: BytesVectored,
    {
        RecvVectored { body: self, bufs }
    }

    /// Read the entire body into `buf`, up to `limit` bytes.
    ///
    /// If the body is larger then `limit` bytes it return an `io::Error`.
    pub async fn read_all(&mut self, buf: &mut Vec<u8>, limit: usize) -> io::Result<()> {
        let mut total = 0;
        loop {
            // Copy bytes in our buffer.
            let bytes = self.buf_bytes();
            let len = bytes.len();
            if limit < total + len {
                return Err(io::Error::new(io::ErrorKind::Other, "body too large"));
            }

            buf.extend_from_slice(bytes);
            self.processed(len);
            total += len;

            let chunk_len = self.chunk_len();
            if chunk_len == 0 {
                match &mut self.kind {
                    // Read all the bytes from the oneshot body.
                    BodyKind::Oneshot { .. } => return Ok(()),
                    // Read all the bytes in the chunk, so need to read another
                    // chunk.
                    BodyKind::Chunked {
                        left_in_chunk,
                        read_complete,
                    } => {
                        if *read_complete {
                            return Ok(());
                        }

                        self.conn.read_chunk(left_in_chunk, read_complete).await?;
                        // Copy read bytes again.
                        continue;
                    }
                }
            }
            // Continue to reading below.
            break;
        }

        loop {
            // Limit the read until the end of the chunk/body.
            let chunk_len = self.chunk_len();
            if chunk_len == 0 {
                return Ok(());
            } else if total + chunk_len > limit {
                return Err(io::Error::new(io::ErrorKind::Other, "body too large"));
            }

            (&mut *buf).reserve(chunk_len);
            self.conn.stream.recv_n(&mut *buf, chunk_len).await?;
            total += chunk_len;

            // FIXME: doesn't deal with chunked bodies.
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

    /// Copy already read bytes.
    ///
    /// Same as [`Body::buf_bytes`] this is limited to the bytes of this
    /// request/chunk, i.e. it doesn't contain the next request/chunk.
    fn copy_buf_bytes(&mut self, dst: &mut [MaybeUninit<u8>]) -> usize {
        let bytes = self.buf_bytes();
        let len = min(bytes.len(), dst.len());
        if len != 0 {
            let _ = MaybeUninit::write_slice(&mut dst[..len], &bytes[..len]);
            self.processed(len);
        }
        len
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

/// The [`Future`] behind [`Body::recv`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Recv<'b, B> {
    body: &'b mut Body<'b>,
    buf: B,
}

impl<'b, B> Future for Recv<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Recv { body, buf } = Pin::into_inner(self);

        let mut len = 0;
        loop {
            // Copy bytes in our buffer.
            len += body.copy_buf_bytes(buf.as_bytes());
            if len != 0 {
                unsafe { buf.update_length(len) };
            }

            let limit = body.chunk_len();
            if limit == 0 {
                match &mut body.kind {
                    // Read all the bytes from the oneshot body.
                    BodyKind::Oneshot { .. } => return Poll::Ready(Ok(len)),
                    // Read all the bytes in the chunk, so need to read another
                    // chunk.
                    BodyKind::Chunked {
                        left_in_chunk,
                        read_complete,
                    } => {
                        ready!(body.conn.try_read_chunk(left_in_chunk, read_complete))?;
                        // Copy read bytes again.
                        continue;
                    }
                }
            }
            // Continue to reading below.
            break;
        }

        // Read from the stream if there is space left.
        if buf.has_spare_capacity() {
            // Limit the read until the end of the chunk/body.
            let limit = body.chunk_len();
            loop {
                match body.conn.stream.try_recv(buf.limit(limit)) {
                    Ok(n) => return Poll::Ready(Ok(len + n)),
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return if len == 0 {
                            Poll::Pending
                        } else {
                            Poll::Ready(Ok(len))
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        } else {
            Poll::Ready(Ok(len))
        }
    }
}

/// The [`Future`] behind [`Body::recv_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvVectored<'b, B> {
    body: &'b mut Body<'b>,
    bufs: B,
}

impl<'b, B> Future for RecvVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let RecvVectored { body, bufs } = Pin::into_inner(self);

        let mut len = 0;
        loop {
            // Copy bytes in our buffer.
            for buf in bufs.as_bufs().as_mut() {
                match body.copy_buf_bytes(buf) {
                    0 => break,
                    n => len += n,
                }
            }
            if len != 0 {
                unsafe { bufs.update_lengths(len) };
            }

            let limit = body.chunk_len();
            if limit == 0 {
                match &mut body.kind {
                    // Read all the bytes from the oneshot body.
                    BodyKind::Oneshot { .. } => return Poll::Ready(Ok(len)),
                    // Read all the bytes in the chunk, so need to read another
                    // chunk.
                    BodyKind::Chunked {
                        left_in_chunk,
                        read_complete,
                    } => {
                        ready!(body.conn.try_read_chunk(left_in_chunk, read_complete))?;
                        // Copy read bytes again.
                        continue;
                    }
                }
            }
            // Continue to reading below.
            break;
        }

        // Read from the stream if there is space left.
        if bufs.has_spare_capacity() {
            // Limit the read until the end of the chunk/body.
            let limit = body.chunk_len();
            loop {
                match body.conn.stream.try_recv_vectored(bufs.limit(limit)) {
                    Ok(n) => return Poll::Ready(Ok(len + n)),
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return if len == 0 {
                            Poll::Pending
                        } else {
                            Poll::Ready(Ok(len))
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        } else {
            Poll::Ready(Ok(len))
        }
    }
}

impl<'a> crate::Body<'a> for Body<'a> {
    fn length(&self) -> BodyLength {
        self.len()
    }
}

mod private {
    use std::future::Future;
    use std::io;
    use std::pin::Pin;
    use std::task::{self, ready, Poll};

    use heph_rt::net::TcpStream;

    use super::{Body, BodyKind};

    #[derive(Debug)]
    pub struct SendBody<'c, 's, 'h> {
        pub(super) body: Body<'c>,
        /// Stream we're writing the body to.
        pub(super) stream: &'s mut TcpStream,
        /// HTTP head for the response.
        pub(super) head: &'h [u8],
    }

    impl<'c, 's, 'h> Future for SendBody<'c, 's, 'h> {
        type Output = io::Result<()>;

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
            let SendBody { body, stream, head } = Pin::into_inner(self);

            // Send the HTTP head first.
            // TODO: try to use vectored I/O on first call.
            while !head.is_empty() {
                match stream.try_send(*head) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => *head = &head[n..],
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            while !body.is_empty() {
                let limit = body.chunk_len();
                let bytes = body.buf_bytes();
                let bytes = if bytes.len() > limit {
                    &bytes[..limit]
                } else {
                    bytes
                };
                // TODO: maybe read first if we have less then N bytes?
                if !bytes.is_empty() {
                    match stream.try_send(bytes) {
                        Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                        Ok(n) => {
                            body.processed(n);
                            continue;
                        }
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                            return Poll::Pending
                        }
                        Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                        Err(err) => return Poll::Ready(Err(err)),
                    }
                    // NOTE: we don't continue here, we always return on start
                    // the next iteration of the loop.
                }

                // Read some more data, or the next chunk.
                match &mut body.kind {
                    BodyKind::Oneshot { .. } => {
                        let _ = ready!(body.conn.try_recv())?;
                    }
                    BodyKind::Chunked {
                        left_in_chunk,
                        read_complete,
                    } => {
                        if *left_in_chunk == 0 {
                            ready!(body.conn.try_read_chunk(left_in_chunk, read_complete))?;
                        } else {
                            let _ = ready!(body.conn.try_recv())?;
                        }
                    }
                }
            }

            Poll::Ready(Ok(()))
        }
    }
}

impl<'c> crate::body::PrivateBody<'c> for Body<'c> {
    type WriteMessage<'s, 'h> = private::SendBody<'c, 's, 'h> where Self: 'c;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'c,
    {
        private::SendBody {
            body: self,
            stream,
            head,
        }
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
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
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
}

impl RequestError {
    /// Returns the proper status code for a given error.
    pub const fn proper_status_code(self) -> StatusCode {
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
            | InvalidChunkSize=> StatusCode::BAD_REQUEST,
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
        }
    }

    /// Returns `true` if the connection should be closed based on the error
    /// (after sending a error response).
    pub const fn should_close(self) -> bool {
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
            | InvalidChunkSize => true,
            UnknownMethod => false,
        }
    }

    /// Returns a response with the [proper status code] set and possibly the
    /// [Connection] header if the [connection should be closed].
    ///
    /// [proper status code]: RequestError::proper_status_code
    /// [Connection]: HeaderName::CONNECTION
    /// [connection should be closed]: RequestError::should_close
    pub fn response(self) -> Response<EmptyBody> {
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
            // SAFETY: request never contain a status, only responses do.
            Status => unreachable!(),
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RequestError::*;
        f.write_str(match self {
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
        })
    }
}

/// The message type used by [`HttpServer`] (and [`TcpServer`]).
///
#[doc(inline)]
pub use heph_rt::net::tcp::server::Message;

/// Error returned by [`HttpServer`] (and [`TcpServer`]).
///
#[doc(inline)]
pub use heph_rt::net::tcp::server::Error;
