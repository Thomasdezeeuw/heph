// TODO: `S: Supervisor` currently uses `TcpStream` as argument due to `ArgMap`.
//       Maybe disconnect `S` from `NA`?
//
// TODO: Continue reading RFC 7230 section 4 Transfer Codings.
//
// TODO: RFC 7230 section 3.4 Handling Incomplete Messages.
//
// TODO: RFC 7230 section 3.3.3 point 5:
// > If the sender closes the connection or the recipient
// > times out before the indicated number of octets are
// > received, the recipient MUST consider the message to be
// > incomplete and close the connection.
//
// TODO: chunked encoding.
// TODO: reading request body.

use std::cmp::min;
use std::fmt;
use std::future::Future;
use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::SystemTime;

use heph::net::{tcp, Bytes, BytesVectored, TcpServer, TcpStream};
use heph::spawn::{ActorOptions, Spawn};
use heph::{actor, rt, Actor, NewActor, Supervisor};
use httparse::EMPTY_HEADER;
use httpdate::HttpDate;

use crate::body::BodyLength;
use crate::{FromBytes, HeaderName, Headers, Method, Request, Response, StatusCode, Version};

/// Maximum size of the header (the start line and the headers).
///
/// RFC 7230 section 3.1.1 recommends "all HTTP senders and recipients support,
/// at a minimum, request-line lengths of 8000 octets."
pub const MAX_HEADER_SIZE: usize = 16384;

/// Maximum number of headers parsed from a single request.
pub const MAX_HEADERS: usize = 64;

/// Minimum amount of bytes read from the connection or the buffer will be
/// grown.
const MIN_READ_SIZE: usize = 4096;

/// Size of the buffer used in [`Connection`].
const BUF_SIZE: usize = 8192;

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
    NA::RuntimeAccess: rt::Access + Spawn<S, ArgMap<NA>, NA::RuntimeAccess>,
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

/// An actor that starts a new actor for each accepted TCP connection.
///
/// TODO: same design as TcpServer.
///
/// This actor can start as a thread-local or thread-safe actor. When using the
/// thread-local variant one actor runs per worker thread which spawns
/// thread-local actors to handle the [`TcpStream`]s. See the first example
/// below on how to run this `TcpServer` as a thread-local actor.
///
/// This actor can also run as thread-safe actor in which case it also spawns
/// thread-safe actors. Note however that using thread-*local* version is
/// recommended. The third example below shows how to run the `TcpServer` as
/// thread-safe actor.
///
/// # Graceful shutdown
///
/// Graceful shutdown is done by sending it a [`Terminate`] message, see below
/// for an example. The TCP server can also handle (shutdown) process signals,
/// see "Example 2 my ip" (in the examples directory of the source code) for an
/// example of that.
///
/// [`Terminate`]: heph::actor::messages::Terminate
///
/// # Examples
///
/// TODO.
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
    NA::RuntimeAccess: rt::Access + Spawn<S, ArgMap<NA>, NA::RuntimeAccess>,
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

    fn name(&self) -> &'static str {
        self.new_actor.name()
    }
}

#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Number of bytes of `buf` that are already parsed.
    parsed_bytes: usize,
    /// The HTTP version of the last request.
    last_version: Option<Version>,
    /// The HTTP method of the last request.
    last_method: Option<Method>,
}

impl Connection {
    /// Create a new `Connection`.
    pub fn new(stream: TcpStream) -> Connection {
        Connection {
            stream,
            buf: Vec::with_capacity(BUF_SIZE),
            parsed_bytes: 0,
            last_version: None,
            last_method: None,
        }
    }

    pub fn peer_addr(&mut self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    pub fn local_addr(&mut self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.stream.set_ttl(ttl)
    }

    pub fn ttl(&mut self) -> io::Result<u32> {
        self.stream.ttl()
    }

    pub fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.stream.set_nodelay(nodelay)
    }

    pub fn nodelay(&mut self) -> io::Result<bool> {
        self.stream.nodelay()
    }

    pub fn keepalive(&self) -> io::Result<bool> {
        self.stream.keepalive()
    }

    pub fn set_keepalive(&self, enable: bool) -> io::Result<()> {
        self.stream.set_keepalive(enable)
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
    /// Most [`RequestError`]s can't be receover from and will need the
    /// connection be closed, see [`RequestError::should_close`]. If the
    /// connection is not closed and `next_request` is called again it will
    /// likely return the same error (but this is not guaranteed).
    ///
    /// Also see the [`Connection::last_request_version`] and
    /// [`Connection::last_request_method`] functions to properly respond to
    /// request errors.
    pub async fn next_request<'a>(
        &'a mut self,
    ) -> io::Result<Result<Option<Request<Body<'a>>>, RequestError>> {
        let mut too_short = 0;
        loop {
            // In case of pipelined requests it could be that while reading a
            // previous request's body it partially read the headers of the next
            // (this) request. To handle this we attempt to parse the request if
            // we have more than zero bytes in the first iteration of the loop.
            if self.buf.len() <= too_short {
                // Receive some more bytes.
                if self.stream.recv(&mut self.buf).await? == 0 {
                    if self.buf.is_empty() {
                        // Read the entire stream, so we're done.
                        return Ok(Ok(None));
                    } else {
                        // Couldn't read any more bytes, but we still have bytes
                        // in the buffer. This means it contains a partial
                        // request.
                        return Ok(Err(RequestError::IncompleteRequest));
                    }
                }
            }

            let mut headers = [EMPTY_HEADER; MAX_HEADERS];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(&self.buf[self.parsed_bytes..]) {
                Ok(httparse::Status::Complete(header_length)) => {
                    self.parsed_bytes += header_length;

                    // SAFETY: all these unwraps are safe because `parse` above
                    // ensures there all `Some`.
                    let method = match req.method.unwrap().parse() {
                        Ok(method) => method,
                        Err(_) => return Ok(Err(RequestError::UnknownMethod)),
                    };
                    self.last_method = Some(method);
                    let path = req.path.unwrap().to_string();
                    let version = map_version(req.version.unwrap());
                    self.last_version = Some(version);

                    // RFC 7230 section 3.3.3 Message Body Length.
                    let mut body_length: Option<usize> = None;
                    let res = Headers::from_httparse_headers(req.headers, |name, value| {
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
                            if let Ok(length) = FromBytes::from_bytes(value) {
                                match body_length.as_mut() {
                                    Some(body_length) if *body_length == length => {}
                                    Some(_) => return Err(RequestError::DifferentContentLengths),
                                    None => body_length = Some(length),
                                }
                            } else {
                                return Err(RequestError::InvalidContentLength);
                            }
                        } else if *name == HeaderName::TRANSFER_ENCODING {
                            todo!("transfer encoding");

                            // TODO: we can support chunked, but for other
                            // encoding we need external packages (for compress,
                            // deflate, gzip).
                            // Not supported transfer-encoding respond with 501
                            // (Not Implemented).
                            //
                            // RFC 7230 section 3.3.3 point 3:
                            // > If a Transfer-Encoding header field is present
                            // > in a request and the chunked transfer coding is
                            // > not the final encoding, the message body length
                            // > cannot be determined reliably; the server MUST
                            // > respond with the 400 (Bad Request) status code
                            // > and then close the connection.
                            // >
                            // > If a message is received with both a
                            // > Transfer-Encoding and a Content-Length header
                            // > field, the Transfer-Encoding overrides the
                            // > Content-Length. [..] A sender MUST remove the
                            // > received Content-Length field prior to
                            // > forwarding such a message downstream.
                        }
                        Ok(())
                    });
                    let headers = match res {
                        Ok(headers) => headers,
                        Err(err) => return Ok(Err(err)),
                    };

                    // TODO: RFC 7230 section 3.3.3:
                    // > A server MAY reject a request that contains a message
                    // > body but not a Content-Length by responding with 411
                    // > (Length Required).
                    // Maybe do this for POST/PUT/etc. that (usually) requires a
                    // body?

                    // RFC 7230 section 3.3.3 point 6:
                    // > If this is a request message and none of the above are
                    // > true, then the message body length is zero (no message
                    // > body is present).
                    let size = body_length.unwrap_or(0);

                    let body = Body {
                        conn: self,
                        left: size,
                    };
                    return Ok(Ok(Some(Request::new(method, path, version, headers, body))));
                }
                Ok(httparse::Status::Partial) => {
                    // Buffer doesn't include the entire request header, try
                    // reading more bytes (in the next iteration).
                    too_short = self.buf.len();
                    self.last_method = req.method.and_then(|m| m.parse().ok());
                    if let Some(version) = req.version {
                        self.last_version = Some(map_version(version));
                    }

                    if too_short >= MAX_HEADER_SIZE {
                        todo!("HTTP request header too large");
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
    /// # todo!();
    ///
    /// // Reading a request returned this error.
    /// let err = RequestError::IncompleteRequest;
    ///
    /// // We can use `last_request_version` to determine the client prefered
    /// // HTTP version, or default to the server prefered version (HTTP/1.1
    /// // here).
    /// let version = conn.last_request_version().unwrap_or(Version::Http11);
    /// let body = format!("Bad request: {}", err);
    /// let body = OneshotBody::new(body.as_bytes());
    /// let response = Response::new(version, StatusCode::BAD_REQUEST, Headers::EMPTY, body);
    ///
    /// // We can use `last_request_method` to determine the method of the last
    /// // request, which is used to determine if we need to send a body.
    /// let request_method = conn.last_request_method().unwrap_or(Method::Get);
    /// // Respond with the response.
    /// conn.send_response(request_method, response);
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

    /// Respond to a request.
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
        headers: Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body<'b>,
    {
        let req_method = self.last_method.unwrap_or(Method::Get);
        let version = self.last_version.unwrap_or(Version::Http11).highest_minor();
        let response = Response::new(version, status, headers, body);
        self.send_response(req_method, response).await
    }

    /// Send a [`Response`].
    ///
    /// # Notes
    ///
    /// This automatically sets the "Content-Length" and "Date" headers if not
    /// provided in `response`.
    ///
    /// If `request_method.`[`expects_body`] or
    /// `response.status().`[`includes_body`] returns false this will not write
    /// the body to the connection.
    ///
    /// [`expects_body`]: Method::expects_body
    /// [`includes_body`]: StatusCode::includes_body
    pub async fn send_response<'b, B>(
        &mut self,
        request_method: Method,
        response: Response<B>,
    ) -> io::Result<()>
    where
        B: crate::Body<'b>,
    {
        let mut itoa_buf = itoa::Buffer::new();

        // Bytes of the (next) request.
        self.clear_buffer();
        let ignore_end = self.buf.len();

        // Format the status-line (RFC 7230 section 3.1.2).
        self.buf
            .extend_from_slice(response.version().as_str().as_bytes());
        self.buf.push(b' ');
        self.buf
            .extend_from_slice(itoa_buf.format(response.status().0).as_bytes());
        // NOTE: we're not sending a reason-phrase, but the space is required
        // before \r\n.
        self.buf.extend_from_slice(b" \r\n");

        // Format the headers (RFC 7230 section 3.2).
        let mut set_content_length_header = false;
        let mut set_date_header = false;
        for header in response.headers().iter() {
            let name = header.name();
            // Field-name:
            self.buf.extend_from_slice(name.as_ref().as_bytes());
            // NOTE: spacing after the colon (`:`) is optional.
            self.buf.extend_from_slice(b": ");
            // Append the header's value.
            // NOTE: `header.value` shouldn't contain CRLF (`\r\n`).
            self.buf.extend_from_slice(header.value());
            self.buf.extend_from_slice(b"\r\n");

            if name == &HeaderName::CONTENT_LENGTH {
                set_content_length_header = true;
            } else if name == &HeaderName::DATE {
                set_date_header = true;
            }
        }

        // Provide the "Date" header if the user didn't.
        if !set_date_header {
            let now = HttpDate::from(SystemTime::now());
            write!(&mut self.buf, "Date: {}\r\n", now).unwrap();
        }

        // Provide the "Conent-Length" header if the user didn't.
        if !set_content_length_header {
            let body_length = match response.body().length() {
                _ if !request_method.expects_body() || !response.status().includes_body() => 0,
                BodyLength::Known(length) => length,
                BodyLength::Chunked => todo!("chunked response body"),
            };

            self.buf.extend_from_slice(b"Content-Length: ");
            self.buf
                .extend_from_slice(itoa_buf.format(body_length).as_bytes());
            self.buf.extend_from_slice(b"\r\n");
        }

        // End of the header.
        self.buf.extend_from_slice(b"\r\n");

        // Write the response to the stream.
        let head = &self.buf[ignore_end..];
        response
            .into_body()
            .write_response(&mut self.stream, head)
            .await?;

        // Remove the response headers from the buffer.
        self.buf.truncate(ignore_end);
        Ok(())
    }

    /// Clear parsed request(s) from the buffer.
    fn clear_buffer(&mut self) {
        if self.buf.len() == self.parsed_bytes {
            // Parsed all bytes in the buffer, so we can clear it.
            self.buf.clear();
            self.parsed_bytes = 0;
        }

        // TODO: move bytes to the start.
    }
}

const fn map_version(version: u8) -> Version {
    match version {
        0 => Version::Http10,
        // RFC 7230 section 2.6:
        // > A server SHOULD send a response version equal to
        // > the highest version to which the server is
        // > conformant that has a major version less than or
        // > equal to the one received in the request.
        // HTTP/1.1 is the highest we support.
        _ => Version::Http11,
    }
}

/// Body of HTTP [`Request`] read from a [`Connection`].
///
/// # Notes
///
/// If the body is not (completely) read it's still removed from the
/// `Connection`.
#[derive(Debug)]
pub struct Body<'a> {
    conn: &'a mut Connection,
    /// Number of unread (by the user) bytes.
    left: usize,
}

impl<'a> Body<'a> {
    /// Returns the length of the body (in bytes) *left*.
    ///
    /// The returned value is based on the "Content-Length" header, or 0 if not
    /// present.
    pub const fn len(&self) -> usize {
        self.left
    }

    /// Returns `true` if the body is completely read (or was empty to begin
    /// with).
    pub const fn is_empty(&self) -> bool {
        self.left == 0
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

    /// Returns the bytes currently in the buffer.
    /// This is limited to the bytes of this request, i.e. it doesn't contain
    fn buf_bytes(&self) -> &[u8] {
        let bytes = &self.conn.buf[self.conn.parsed_bytes..];
        if bytes.len() > self.left {
            &bytes[..self.left]
        } else {
            bytes
        }
    }

    /// Copy already read bytes.
    fn copy_buf_bytes(&mut self, dst: &mut [MaybeUninit<u8>]) -> usize {
        let bytes = self.buf_bytes();
        let len = bytes.len();
        if len != 0 {
            let len = min(len, dst.len());
            MaybeUninit::write_slice(&mut dst[..len], &bytes[..len]);
            self.processed(len);
            len
        } else {
            0
        }
    }

    /// Mark `n` bytes are processed.
    fn processed(&mut self, n: usize) {
        self.left -= n;
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

        // Copy already read bytes.
        let len = body.copy_buf_bytes(buf.as_bytes());
        if len != 0 {
            unsafe { buf.update_length(len) };
        }

        // Read from the stream if there is space left.
        if buf.has_spare_capacity() {
            loop {
                match body.conn.stream.try_recv(&mut *buf) {
                    Ok(n) => return Poll::Ready(Ok(len + n)),
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        if len != 0 {
                            return Poll::Ready(Ok(len));
                        } else {
                            return Poll::Pending;
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        }
        Poll::Ready(Ok(len))
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

        // Copy already read bytes.
        let mut len = 0;
        for buf in bufs.as_bufs().as_mut() {
            match body.copy_buf_bytes(buf) {
                0 => break,
                n => len += n,
            }
        }
        if len != 0 {
            unsafe { bufs.update_lengths(len) };
        }

        // Read from the stream if there is space left.
        if bufs.has_spare_capacity() {
            loop {
                match body.conn.stream.try_recv_vectored(&mut *bufs) {
                    Ok(n) => return Poll::Ready(Ok(len + n)),
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        if len != 0 {
                            return Poll::Ready(Ok(len));
                        } else {
                            return Poll::Pending;
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        }
        Poll::Ready(Ok(len))
    }
}

impl<'a> crate::Body<'a> for Body<'a> {
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.left)
    }
}

mod private {
    use std::future::Future;
    use std::io;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use heph::net::TcpStream;

    use super::{Body, MIN_READ_SIZE};

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
                match stream.try_send(head) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => *head = &head[n..],
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            while body.left != 0 {
                let bytes = body.buf_bytes();
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
                }

                // Ensure we have space in the buffer to read into.
                body.conn.clear_buffer();
                body.conn.buf.reserve(MIN_READ_SIZE);
                match body.conn.stream.try_recv(&mut body.conn.buf) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                    // Continue to sending the bytes above.
                    Ok(_) => continue,
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            Poll::Ready(Ok(()))
        }
    }
}

impl<'c> crate::body::PrivateBody<'c> for Body<'c> {
    type WriteBody<'s, 'h> = private::SendBody<'c, 's, 'h>;

    fn write_response<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteBody<'s, 'h>
    where
        'c: 'h,
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

        let ignored_len = self.conn.parsed_bytes + self.left;
        if self.conn.buf.len() >= ignored_len {
            // Entire body was already read we can skip the bytes.
            self.conn.parsed_bytes = ignored_len;
            return;
        }

        // TODO: mark more bytes as ignored in `Connection`.
        todo!("ignore the body: read more bytes")
    }
}

/// Error parsing HTTP request.
#[derive(Copy, Clone, Debug)]
pub enum RequestError {
    /// Missing part of request.
    IncompleteRequest,
    /// HTTP Header is too large.
    ///
    /// Limit is defined by [`MAX_HEADER_SIZE`].
    HeaderTooLarge,
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
    /// Invalid byte where token is required.
    InvalidToken,
    /// Invalid byte in new line.
    InvalidNewLine,
    /// Invalid byte in HTTP version.
    InvalidVersion,
    /// Unknown HTTP method, not in [`Method`].
    UnknownMethod,
}

impl RequestError {
    /// Returns the proper status code for a given error.
    pub const fn proper_status_code(self) -> StatusCode {
        use RequestError::*;
        // See the parsing code for various references to the RFC(s) that
        // determine the values here.
        match self {
            IncompleteRequest
            | HeaderTooLarge
            | InvalidContentLength
            | DifferentContentLengths
            | InvalidHeaderName
            | InvalidHeaderValue
            | TooManyHeaders
            | InvalidToken
            | InvalidNewLine
            | InvalidVersion => StatusCode::BAD_REQUEST,
            // RFC 7231 section 4.1:
            // > When a request method is received that is unrecognized or not
            // > implemented by an origin server, the origin server SHOULD
            // > respond with the 501 (Not Implemented) status code.
            UnknownMethod => StatusCode::NOT_IMPLEMENTED,
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
            | HeaderTooLarge
            | InvalidContentLength
            | DifferentContentLengths
            | InvalidHeaderName
            | InvalidHeaderValue
            | TooManyHeaders
            | InvalidToken
            | InvalidNewLine
            | InvalidVersion => true,
            UnknownMethod => false,
        }
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use RequestError::*;
        f.write_str(match self {
            IncompleteRequest => "incomplete request",
            HeaderTooLarge => "header too large",
            InvalidContentLength => "invalid Content-Length header",
            DifferentContentLengths => "different Content-Length headers",
            InvalidHeaderName => "invalid header name",
            InvalidHeaderValue => "invalid header value",
            TooManyHeaders => "too many header",
            InvalidToken | InvalidNewLine => "invalid request syntax",
            InvalidVersion => "invalid version",
            UnknownMethod => "unknown method",
        })
    }
}

/// The message type used by [`HttpServer`] (and [`TcpServer`]).
///
#[doc(inline)]
pub use heph::net::tcp::server::Message;

/// Error returned by [`HttpServer`] (and [`TcpServer`]).
///
#[doc(inline)]
pub use heph::net::tcp::server::Error;
