//! HTTP server.

// TODO: Continue reading RFC 7230 section 4 Transfer Codings.
//
// TODO: RFC 7230 section 3.3.3 point 5:
// > If the sender closes the connection or the recipient
// > times out before the indicated number of octets are
// > received, the recipient MUST consider the message to be
// > incomplete and close the connection.
use std::fmt;
use std::future::Future;
use std::io::{self, Write};
use std::mem::{MaybeUninit, take};
use std::net::SocketAddr;
use std::time::SystemTime;

use heph::supervisor::Supervisor;
use heph::{NewActor, actor};
use heph_rt::extract::Extract;
use heph_rt::fd::AsyncFd;
use heph_rt::io::{BufMut, BufMutSlice};
use heph_rt::spawn::Spawn;
use heph_rt::spawn::options::{ActorOptions, InboxSize};
use heph_rt::timer::DeadlinePassed;
use heph_rt::{Access, net};
use httpdate::HttpDate;

use crate::body::{BodyLength, EmptyBody};
use crate::head::header::{FromHeaderValue, Header, HeaderName, Headers};
use crate::{
    BUF_SIZE, INIT_HEAD_SIZE, MAX_HEAD_SIZE, MAX_HEADERS, MIN_READ_SIZE, Method, Request, Response,
    StatusCode, Version, map_version_byte, set_nodelay, trim_ws,
};

pub mod handler;
use handler::{DefaultErrorHandler, Handler, HandlerSupervisor, HttpHandle, HttpHandleError};

/// HTTP server.
///
/// The HTTP server uses the same design as [`net::Server`], see that for how
/// this type is supposed to work, including graceful shutdown.
///
/// In additional to the usual design of spawning a single actor per incoming
/// connection this also allows the use of request handlers.
#[derive(Clone, Debug)]
pub struct Server<S, NA>(net::Server<S, NA>);

impl<S, NA> Server<S, NA> {
    /// Create a new HTTP server.
    ///
    /// The actor needs to fully handle a single [`Connection`]. This allows the
    /// incoming requests in whatever way you see fit. Also see
    /// [`Server::new_using_handler`] for a HTTP server that only needs a
    /// function to handle a single request at a time, which is easier to use.
    ///
    /// Arguments:
    ///  * `address`: the address to listen on,
    ///  * `supervisor`: the [`Supervisor`] used to supervise each started actor,
    ///  * `new_actor`: the [`NewActor`] implementation to start each actor, and
    ///  * `options`: the actor options used to spawn the new actors.
    pub fn new(
        address: SocketAddr,
        supervisor: S,
        new_actor: NA,
        options: ActorOptions,
    ) -> io::Result<Server<S, NewConnection<NA>>>
    where
        S: Supervisor<NewConnection<NA>> + Clone + 'static,
        NA: NewActor<Argument = Connection> + Clone + 'static,
        NA::RuntimeAccess: Access + Spawn<S, NewConnection<NA>, NA::RuntimeAccess>,
    {
        let new_actor = NewConnection(new_actor);
        net::Server::new(address, supervisor, new_actor, options).map(Server)
    }

    /// Create a new HTTP server that uses a single `Future` to handle incoming
    /// connection.
    ///
    /// Arguments:
    ///  * `address`: the address to listen on,
    ///  * `handler`: the future that is used to process a single request.
    ///
    /// Setting additional options, such as the error handler, can be done using
    /// the [impl block] below.
    ///
    /// [impl block]: #impl-Server<S,+Handler<H,+E,+RT>>
    pub fn new_using_handler<H, RT>(
        address: SocketAddr,
        handler: H,
    ) -> io::Result<Server<HandlerSupervisor, Handler<H, DefaultErrorHandler, RT>>>
    where
        H: HttpHandle + Clone + 'static,
        RT: Access
            + Spawn<HandlerSupervisor, Handler<H, DefaultErrorHandler, RT>, RT>
            + Clone
            + 'static,
    {
        let new_actor = Handler::new(handler, DefaultErrorHandler);
        let options = ActorOptions::default().with_inbox_size(InboxSize::ONE);
        net::Server::new(address, HandlerSupervisor, new_actor, options).map(Server)
    }

    /// Returns the address the server is bound to.
    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }
}

/// Set optional configuration options when using the HTTP handler.
///
/// See [`Server::new_using_handler`] for creation.
impl<S, H, E, RT> Server<S, Handler<H, E, RT>>
where
    H: HttpHandle + Clone + 'static,
    E: HttpHandleError + Clone + 'static,
{
    /// Change the supervisor for the actor that handles the connections.
    ///
    /// Defaults to [`HandlerSupervisor`].
    pub fn with_supervisor<S2>(self, supervisor: S2) -> Server<S2, Handler<H, E, RT>>
    where
        S2: Supervisor<Handler<H, E, RT>> + Clone + 'static,
    {
        Server(self.0.map_supervisor(|_| supervisor))
    }

    /// Change the error handler.
    ///
    /// Defaults to [`DefaultErrorHandler`].
    pub fn with_error_handler<E2>(self, error_handler: E2) -> Server<S, Handler<H, E2, RT>>
    where
        E2: HttpHandleError + Clone + 'static,
        RT: Access + Spawn<S, Handler<H, E2, RT>, RT> + Clone + 'static,
    {
        Server(self.0.map_actor(|h| h.with_error_handler(error_handler)))
    }

    /// Change the actor options for the actor that handles incoming connections.
    pub fn with_actor_options<F>(self, options: ActorOptions) -> Self {
        Server(self.0.map_actor_options(|_| options))
    }
}

impl<S, NA> NewActor for Server<S, NA>
where
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = AsyncFd> + Clone + 'static,
    NA::RuntimeAccess: Access + Spawn<S, NA, NA::RuntimeAccess>,
{
    type Message = ServerMessage;
    type Argument = ();
    type Actor = impl Future<Output = Result<(), ServerError<NA::Error>>>;
    type Error = io::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        (): Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        self.0.new(ctx, ())
    }
}

/// `NewActor` mapper used by [`Server::new`].
///
/// This accepts an [`AsyncFd`] as arguments, sets TCP no delay on it and
/// converts it into a [`Connection`]. Then it passes that connection on to the
/// provided `NewActor` in `NA`.
#[derive(Debug, Clone)]
pub struct NewConnection<NA>(NA);

impl<NA: NewActor<Argument = Connection>> NewActor for NewConnection<NA> {
    type Message = NA::Message;
    type Argument = AsyncFd;
    type Actor = NA::Actor;
    type Error = NA::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        fd: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        if let Some(fd) = fd.as_fd() {
            if let Err(err) = set_nodelay(fd) {
                log::warn!("failed to set TCP no delay on accepted connection: {err}");
            }
        }

        let conn = Connection::new(fd);
        self.0.new(ctx, conn)
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
/// [HTTP requests]: Request
/// [HTTP responses]: crate::Response
#[derive(Debug)]
pub struct Connection {
    stream: AsyncFd,
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
    fn new(stream: AsyncFd) -> Connection {
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

            let mut headers = const { [MaybeUninit::uninit(); MAX_HEADERS] };
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
                                            return Err(RequestError::DifferentContentLengths);
                                        }
                                        Some(BodyLength::Chunked) => {
                                            return Err(
                                                RequestError::ContentLengthAndTransferEncoding,
                                            );
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
    /// let mut conn: Connection = /* From Server. */
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
            self.stream.send_all(http_head).extract().await?
        };

        if self.buf.is_empty() {
            // We used the read buffer so let's put it back.
            http_head.clear();
            self.buf = http_head;
        }

        Ok(())
    }

    /// See [`AsyncFd::peer_addr`].
    pub async fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr().await
    }

    /// See [`AsyncFd::local_addr`].
    pub async fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr().await
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
            self.processed((buf.spare_capacity() - len_before) as usize);
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
            let (limited_bufs, _) = self.conn.stream.recv_vectored(bufs.limit(limit)).await?;
            let bufs = limited_bufs.into_inner();
            self.processed((bufs.total_spare_capacity() - len_before) as usize);
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
                    // FIXME: add some sort of unprocessed field to Connection
                    // that handles skipping over bytes when reading the next
                    // request.
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
        // See the parsing code for various references to the RFC(s) that
        // determine the values here.
        match self {
            RequestError::IncompleteRequest
            | RequestError::HeadTooLarge
            | RequestError::InvalidContentLength
            | RequestError::DifferentContentLengths
            | RequestError::InvalidHeaderName
            | RequestError::InvalidHeaderValue
            | RequestError::TooManyHeaders
            | RequestError::ChunkedNotLastTransferEncoding
            | RequestError::ContentLengthAndTransferEncoding
            | RequestError::InvalidToken
            | RequestError::InvalidNewLine
            | RequestError::InvalidVersion
            | RequestError::InvalidChunkSize => StatusCode::BAD_REQUEST,
            // RFC 7230 section 3.3.1:
            // > A server that receives a request message with a transfer coding
            // > it does not understand SHOULD respond with 501 (Not
            // > Implemented).
            RequestError::UnsupportedTransferEncoding
            // RFC 7231 section 4.1:
            // > When a request method is received that is unrecognized or not
            // > implemented by an origin server, the origin server SHOULD
            // > respond with the 501 (Not Implemented) status code.
            | RequestError::UnknownMethod => StatusCode::NOT_IMPLEMENTED,
            RequestError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns `true` if the connection should be closed based on the error
    /// (after sending a error response).
    pub const fn should_close(&self) -> bool {
        // See the parsing code for various references to the RFC(s) that
        // determine the values here.
        match self {
            RequestError::IncompleteRequest
            | RequestError::HeadTooLarge
            | RequestError::InvalidContentLength
            | RequestError::DifferentContentLengths
            | RequestError::InvalidHeaderName
            | RequestError::InvalidHeaderValue
            | RequestError::UnsupportedTransferEncoding
            | RequestError::TooManyHeaders
            | RequestError::ChunkedNotLastTransferEncoding
            | RequestError::ContentLengthAndTransferEncoding
            | RequestError::InvalidToken
            | RequestError::InvalidNewLine
            | RequestError::InvalidVersion
            | RequestError::InvalidChunkSize
            | RequestError::Io(_) => true,
            RequestError::UnknownMethod => false,
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

    // NOTE: not implemented using the From trait because it's not part of the
    // public API.
    fn from_httparse(err: httparse::Error) -> RequestError {
        match err {
            httparse::Error::HeaderName => RequestError::InvalidHeaderName,
            httparse::Error::HeaderValue => RequestError::InvalidHeaderValue,
            httparse::Error::Token => RequestError::InvalidToken,
            httparse::Error::NewLine => RequestError::InvalidNewLine,
            httparse::Error::Version => RequestError::InvalidVersion,
            httparse::Error::TooManyHeaders => RequestError::TooManyHeaders,
            // Requests never contain a status, only responses do, but we don't
            // want a panic branch (from `unreachable!`) here.
            httparse::Error::Status => RequestError::IncompleteRequest,
        }
    }

    /// Returns a static error message for the error.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestError::IncompleteRequest => "incomplete request",
            RequestError::HeadTooLarge => "head too large",
            RequestError::InvalidContentLength => "invalid Content-Length header",
            RequestError::DifferentContentLengths => "different Content-Length headers",
            RequestError::InvalidHeaderName => "invalid header name",
            RequestError::InvalidHeaderValue => "invalid header value",
            RequestError::TooManyHeaders => "too many header",
            RequestError::UnsupportedTransferEncoding => "unsupported Transfer-Encoding",
            RequestError::ChunkedNotLastTransferEncoding => "invalid Transfer-Encoding header",
            RequestError::ContentLengthAndTransferEncoding => {
                "provided both Content-Length and Transfer-Encoding headers"
            }
            RequestError::InvalidToken | RequestError::InvalidNewLine => "invalid request syntax",
            RequestError::InvalidVersion => "invalid version",
            RequestError::UnknownMethod => "unknown method",
            RequestError::InvalidChunkSize => "invalid chunk size",
            RequestError::Io(_) => "I/O error",
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

#[doc(no_inline)]
pub use heph_rt::net::{ServerError, ServerMessage};
