// TODO: `S: Supervisor` currently uses `TcpStream` as argument due to `ArgMap`.
//       Maybe disconnect `S` from `NA`?
//
// TODO: Continue reading RFC 7230 section 4 Transfer Codings.
//
// TODO: chunked encoding.
// TODO: reading request body.

use std::fmt;
use std::io::{self, IoSlice, Write};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::net::{tcp, TcpServer, TcpStream};
use heph::spawn::{ActorOptions, Spawn};
use heph::{actor, rt, Actor, NewActor, Supervisor};
use httparse::EMPTY_HEADER;

use crate::{FromBytes, HeaderName, Headers, Method, Request, Response, StatusCode, Version};

/// Maximum size of the header (the start line and the headers).
///
/// RFC 7230 section 3.1.1 recommends ``all HTTP senders and recipients support,
/// at a minimum, request-line lengths of 8000 octets.''
pub const MAX_HEADER_SIZE: usize = 16384;

/// Maximum number of headers parsed for each request.
pub const MAX_HEADERS: usize = 64;

/// Minimum amount of bytes read from the connection or the buffer will be
/// grown.
#[allow(dead_code)] // FIXME: use this in reading.
const MIN_READ_SIZE: usize = 512;

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
/// Maps `NA` to accept `(TcpStream, SocketAddr)` as argument.
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
    /// Next is a `Result<Option<`[`Request`]`>, `[`RequestError`]`>`. `None`
    /// is returned if the connections contains no more requests, i.e. all bytes
    /// are read. If the connection contains a request it will return a
    /// [`Request`]. If the request is somehow invalid/incomplete it will return
    /// an [`RequestError`].
    ///
    /// # Notes
    ///
    /// Most [`RequestError`]s can't be receover from and will need the
    /// connection be closed, see [`RequestError::should_close`]. If the
    /// connection is not closed and [`next_request`] is called again it will
    /// likely return the same error (but this is not guaranteed).
    ///
    /// [`next_request`]: Connection::next_request
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
                        // Couldn't read any more bytes, but we still have bytes in
                        // the buffer. This means it contains a partial request.
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
                        size,
                        left: size,
                    };
                    return Ok(Ok(Some(Request {
                        method,
                        version,
                        path,
                        headers,
                        body,
                    })));
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
    /// use heph_http::{Response, Headers, StatusCode, Version};
    /// use heph_http::server::{Connection, RequestError};
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
    /// let response = Response::new(version, StatusCode::BAD_REQUEST, Headers::EMPTY, body);
    ///
    /// // Respond with the response.
    /// conn.respond(response);
    ///
    /// // Close the connection if the error is fatal.
    /// if err.should_close() {
    ///     conn.close();
    ///     return;
    /// }
    /// # }
    /// ```
    pub fn last_request_version(&self) -> Option<Version> {
        self.last_version
    }

    /// # Notes
    ///
    /// This automatically sets the "Content-Length" header if no provided in
    /// `response`.
    ///
    /// This doesn't include the body if the response is to a HEAD request.
    pub async fn respond<B>(&mut self, response: Response<B>) -> io::Result<()>
    where
        B: crate::Body,
    {
        // Bytes of the (next) request.
        self.clear_buffer();
        let ignore_end = self.buf.len();

        // TODO: RFC 7230 section 3.3:
        // > The presence of a message body in a response depends on
        // > both the request method to which it is responding and
        // > the response status code (Section 3.1.2). Responses to
        // > the HEAD request method (Section 4.3.2 of [RFC7231])
        // > never include a message body because the associated
        // > response header fields (e.g., Transfer-Encoding,
        // > Content-Length, etc.), if present, indicate only what
        // > their values would have been if the request method had
        // > been GET (Section 4.3.1 of [RFC7231]). 2xx (Successful)
        // > responses to a CONNECT request method (Section 4.3.6 of
        // > [RFC7231]) switch to tunnel mode instead of having a
        // > message body. All 1xx (Informational), 204 (No
        // > Content), and 304 (Not Modified) responses do not
        // > include a message body. All other responses do include
        // > a message body, although the body might be of zero
        // > length.

        // Format the status-line (RFC 7230 section 3.1.2).
        // NOTE: we're not sending a reason-phrase, but the space is required
        // before \r\n.
        write!(
            &mut self.buf,
            "{} {} \r\n",
            response.version, response.status
        )
        .unwrap();

        // Format the headers (RFC 7230 section 3.2).
        let mut set_content_length_header = false;
        for header in response.headers.iter() {
            // Field-name:
            // NOTE: spacing after the colon (`:`) is optional.
            write!(&mut self.buf, "{}: ", header.name()).unwrap();
            // Append the header's value.
            // NOTE: `header.value` shouldn't contain CRLF (`\r\n`).
            self.buf.extend_from_slice(header.value());
            self.buf.extend_from_slice(b"\r\n");
            if *header.name() == HeaderName::CONTENT_LENGTH {
                set_content_length_header = true;
            }
        }

        // Response body.
        let body = if let Some(Method::Head) = self.last_method {
            // RFC 7231 section 4.3.2:
            // > The HEAD method is identical to GET except that the server MUST
            // > NOT send a message body in the response (i.e., the response
            // > terminates at the end of the header section).
            &[]
        } else {
            response.body.as_bytes()
        };

        // Provide the "Conent-Length" if the user didn't.
        if !set_content_length_header {
            write!(&mut self.buf, "Content-Length: {}\r\n", body.len()).unwrap();
        }

        // End of the header.
        self.buf.extend_from_slice(b"\r\n");

        // Write the response to the connection.
        let header = IoSlice::new(&self.buf[ignore_end..]);
        let body = IoSlice::new(body);
        self.stream.send_vectored_all(&mut [header, body]).await?;

        // Remove the response from the buffer.
        self.buf.truncate(ignore_end);
        Ok(())
    }

    /// Close the connection.
    ///
    /// This should be called in case of certain [`RequestError`]s, see
    /// [`RequestError::should_close`]. It should also be called if a response
    /// it returned without a length, that is a response with a Content-Length
    /// header and not using chunked transfer encoding.
    pub fn close(self) {
        drop(self);
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
pub struct Body<'a> {
    conn: &'a mut Connection,
    /// Total size of the HTTP body.
    size: usize,
    /// Number of unread (by the user) bytes.
    left: usize,
}

impl<'a> Body<'a> {
    // TODO: RFC 7230 section 3.4 Handling Incomplete Messages.

    // TODO: RFC 7230 section 3.3.3 point 5:
    // > If the sender closes the connection or the recipient
    // > times out before the indicated number of octets are
    // > received, the recipient MUST consider the message to be
    // > incomplete and close the connection.

    /// Returns the size of the body in bytes.
    ///
    /// The returned value is based on the "Content-Length" header, or 0 if not
    /// present.
    pub fn len(&self) -> usize {
        // TODO: chunked encoding.
        self.size
    }

    /// Returns the number of bytes left in the body.
    ///
    /// See [`Body::len`].
    pub fn left(&self) -> usize {
        self.left
    }

    /// Ignore the body, but removes it from the connection.
    pub fn ignore(&mut self) -> io::Result<()> {
        if self.size == 0 {
            // Empty body, then we're done quickly.
            return Ok(());
        }

        let ignored_len = self.conn.parsed_bytes + self.size;
        if self.conn.buf.len() >= ignored_len {
            // Entire body was already read we can skip the bytes.
            self.conn.parsed_bytes = ignored_len;
            return Ok(());
        }

        // TODO: read more bytes from the stream.
        todo!("ignore the body: read more bytes")
        // NOTE: conn.clear_buffer
    }
}

/* TODO: read entire body? maybe an assertion?
impl<'a> Drop for Body<'a> {
    fn drop(&mut self) {
        todo!()
    }
}
*/

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
    pub fn proper_status_code(self) -> StatusCode {
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
    pub fn should_close(self) -> bool {
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

/// The message type used by [`HttpServer`].
///
/// The message implements [`From`]`<`[`Terminate`]`>` and
/// [`TryFrom`]`<`[`Signal`]`>` for the message, allowing for graceful shutdown.
pub use heph::net::tcp::server::Message;

/// Error returned by the [`HttpServer`] actor.
pub use heph::net::tcp::server::Error;
