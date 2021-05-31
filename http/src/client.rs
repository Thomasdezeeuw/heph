//! Module with the HTTP client implementation.

// FIXME: remove.
#![allow(missing_docs)]

use std::cmp::min;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, io};

use heph::net::tcp::stream::{self, TcpStream};
use heph::{actor, rt};

use crate::body::{BodyLength, EmptyBody};
use crate::header::{FromHeaderValue, HeaderName, Headers};
use crate::{
    map_version_byte, trim_ws, Method, Response, StatusCode, BUF_SIZE, MAX_HEADERS, MAX_HEAD_SIZE,
    MIN_READ_SIZE,
};

#[derive(Debug)]
pub struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Number of bytes of `buf` that are already parsed.
    /// NOTE: this may be larger then `buf.len()`, in which case a `Body` was
    /// dropped without reading it entirely.
    parsed_bytes: usize,
}

impl Client {
    /// Create a new HTTP client, connected to `address`.
    pub fn connect<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        address: SocketAddr,
    ) -> io::Result<Connect>
    where
        RT: rt::Access,
    {
        TcpStream::connect(ctx, address).map(|connect| Connect { connect })
    }

    /// Send a GET request.
    ///
    /// # Notes
    ///
    /// Any [`ResponseError`] are turned into [`io::Error`]. If you want to
    /// handle the `ResponseError`s separately use [`Client::request`].
    pub async fn get<'c, 'p>(&'c mut self, path: &'p str) -> io::Result<Response<Body<'c>>> {
        let res = self
            .request(Method::Get, path, &Headers::EMPTY, EmptyBody)
            .await;
        match res {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(err)) => Err(err.into()),
            Err(err) => Err(err),
        }
    }

    /// Make a [`Request`] and wait (non-blocking) for a [`Response`].
    ///
    /// [`Request`]: crate::Request
    ///
    /// # Notes
    ///
    /// This always uses HTTP/1.1 to make the requests.
    ///
    /// If the server doesn't respond this return an [`io::Error`] with
    /// [`io::ErrorKind::UnexpectedEof`].
    #[allow(clippy::future_not_send)] // TODO.
    pub async fn request<'c, 'b, B>(
        &'c mut self,
        method: Method,
        path: &str,
        headers: &Headers,
        body: B,
    ) -> io::Result<Result<Response<Body<'c>>, ResponseError>>
    where
        B: crate::Body<'b>,
    {
        self.send_request(method, path, headers, body).await?;
        match self.read_response(method).await {
            Ok(Ok(Some(request))) => Ok(Ok(request)),
            Ok(Ok(None)) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "no HTTP response",
            )),
            Ok(Err(err)) => Ok(Err(err)),
            Err(err) => Err(err),
        }
    }

    #[allow(clippy::future_not_send)] // TODO.
    pub async fn send_request<'b, B>(
        &mut self,
        method: Method,
        path: &str,
        headers: &Headers,
        body: B,
    ) -> io::Result<()>
    where
        B: crate::Body<'b>,
    {
        // Clear bytes from the previous request, keeping the bytes of the
        // response.
        self.clear_buffer();
        let ignore_end = self.buf.len();

        // Request line.
        self.buf.extend_from_slice(method.as_str().as_bytes());
        self.buf.push(b' ');
        self.buf.extend_from_slice(path.as_bytes());
        self.buf.extend_from_slice(b" HTTP/1.1\r\n");

        // Headers.
        let mut set_user_agent_header = false;
        let mut set_content_length_header = false;
        let mut set_transfer_encoding_header = false;
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

            if name == &HeaderName::USER_AGENT {
                set_user_agent_header = true;
            } else if name == &HeaderName::CONTENT_LENGTH {
                set_content_length_header = true;
            } else if name == &HeaderName::TRANSFER_ENCODING {
                set_transfer_encoding_header = true;
            }
        }

        /* TODO: set "Host" header.
        // Provide the "Host" header if the user didn't.
        if !set_host_header {
            write!(&mut self.buf, "Host: {}\r\n", self.host).unwrap();
        }
        */

        // Provide the "User-Agent" header if the user didn't.
        if !set_user_agent_header {
            self.buf.extend_from_slice(
                concat!("User-Agent: Heph-HTTP/", env!("CARGO_PKG_VERSION"), "\r\n").as_bytes(),
            );
        }

        if !set_content_length_header && !set_transfer_encoding_header {
            match body.length() {
                BodyLength::Known(0) => {} // No need for a "Content-Length" header.
                BodyLength::Known(length) => {
                    let mut itoa_buf = itoa::Buffer::new();
                    self.buf.extend_from_slice(b"Content-Length: ");
                    self.buf
                        .extend_from_slice(itoa_buf.format(length).as_bytes());
                    self.buf.extend_from_slice(b"\r\n");
                }
                BodyLength::Chunked => {
                    self.buf
                        .extend_from_slice(b"Transfer-Encoding: chunked\r\n");
                }
            }
        }

        // End of the HTTP head.
        self.buf.extend_from_slice(b"\r\n");

        // Write the request to the stream.
        let http_head = &self.buf[ignore_end..];
        body.write_message(&mut self.stream, http_head).await?;

        // Remove the request from the buffer.
        self.buf.truncate(ignore_end);
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // TODO.
    pub async fn read_response<'a>(
        &'a mut self,
        request_method: Method,
    ) -> io::Result<Result<Option<Response<Body<'a>>>, ResponseError>> {
        let mut too_short = 0;
        loop {
            // In case of pipelined responses it could be that while reading a
            // previous response's body it partially read the head of the next
            // (this) response. To handle this we first attempt to parse the
            // response if we have more than zero bytes (of the next response)
            // in the first iteration of the loop.
            while self.parsed_bytes >= self.buf.len() || self.buf.len() <= too_short {
                // While we didn't read the entire previous response body, or
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
                        // response.
                        Ok(Err(ResponseError::IncompleteResponse))
                    };
                }
            }

            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            let mut response = httparse::Response::new(&mut headers);
            // SAFETY: because we received until at least `self.parsed_bytes >=
            // self.buf.len()` above, we can safely slice the buffer..
            match response.parse(&self.buf[self.parsed_bytes..]) {
                Ok(httparse::Status::Complete(head_length)) => {
                    self.parsed_bytes += head_length;

                    // SAFETY: all these unwraps are safe because `parse` above
                    // ensures there all `Some`.
                    let version = map_version_byte(response.version.unwrap());
                    let status = StatusCode(response.code.unwrap());
                    // NOTE: don't care about the reason.

                    // RFC 7230 section 3.3.3 Message Body Length.
                    let mut body_length: Option<ResponseBodyLength> = None;
                    let res = Headers::from_httparse_headers(response.headers, |name, value| {
                        if *name == HeaderName::CONTENT_LENGTH {
                            // RFC 7230 section 3.3.3 point 4:
                            // > If a message is received without
                            // > Transfer-Encoding and with either multiple
                            // > Content-Length header fields having differing
                            // > field-values or a single Content-Length header
                            // > field having an invalid value, then the message
                            // > framing is invalid and the recipient MUST treat
                            // > it as an unrecoverable error. [..] If this is a
                            // > response message received by a user agent, the
                            // > user agent MUST close the connection to the
                            // > server and discard the received response.
                            if let Ok(length) = FromHeaderValue::from_bytes(value) {
                                match body_length.as_mut() {
                                    Some(ResponseBodyLength::Known(body_length))
                                        if *body_length == length => {}
                                    Some(ResponseBodyLength::Known(_)) => {
                                        return Err(ResponseError::DifferentContentLengths)
                                    }
                                    Some(
                                        ResponseBodyLength::Chunked | ResponseBodyLength::ReadToEnd,
                                    ) => {
                                        return Err(ResponseError::ContentLengthAndTransferEncoding)
                                    }
                                    // RFC 7230 section 3.3.3 point 5:
                                    // > If a valid Content-Length header field
                                    // > is present without Transfer-Encoding,
                                    // > its decimal value defines the expected
                                    // > message body length in octets.
                                    None => body_length = Some(ResponseBodyLength::Known(length)),
                                }
                            } else {
                                return Err(ResponseError::InvalidContentLength);
                            }
                        } else if *name == HeaderName::TRANSFER_ENCODING {
                            let mut encodings = value.split(|b| *b == b',').peekable();
                            while let Some(encoding) = encodings.next() {
                                match trim_ws(encoding) {
                                    b"chunked" => {
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
                                                ResponseError::ContentLengthAndTransferEncoding,
                                            );
                                        }

                                        // RFC 7230 section 3.3.3 point 3:
                                        // > If a Transfer-Encoding header field
                                        // > is present in a response and the
                                        // > chunked transfer coding is not the
                                        // > final encoding, the message body
                                        // > length is determined by reading the
                                        // > connection until it is closed by
                                        // > the server.
                                        if encodings.peek().is_some() {
                                            body_length = Some(ResponseBodyLength::ReadToEnd)
                                        } else {
                                            body_length = Some(ResponseBodyLength::Chunked);
                                        }
                                    }
                                    b"identity" => {} // No changes.
                                    // TODO: support "compress", "deflate" and
                                    // "gzip".
                                    _ => return Err(ResponseError::UnsupportedTransferEncoding),
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
                        // RFC 7230 section 3.3.3 point 2:
                        // > Any 2xx (Successful) response to a CONNECT request
                        // > implies that the connection will become a tunnel
                        // > immediately after the empty line that concludes the
                        // > header fields. A client MUST ignore any
                        // > Content-Length or Transfer-Encoding header fields
                        // > received in such a message.
                        _ if matches!(request_method, Method::Connect)
                            && status.is_successful() =>
                        {
                            BodyKind::Known { left: 0 }
                        }
                        Some(ResponseBodyLength::Known(left)) => BodyKind::Known { left },
                        Some(ResponseBodyLength::Chunked) => {
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
                                Err(_) => return Ok(Err(ResponseError::InvalidChunkSize)),
                            }
                        }
                        Some(ResponseBodyLength::ReadToEnd) => BodyKind::Unknown,
                        // RFC 7230 section 3.3.3 point 1:
                        // > Any response to a HEAD request and any response
                        // > with a 1xx (Informational), 204 (No Content), or
                        // > 304 (Not Modified) status code is always terminated
                        // > by the first empty line after the header fields,
                        // > regardless of the header fields present in the
                        // > message, and thus cannot contain a message body.
                        // NOTE: we don't follow this strictly as a server might
                        // not be implemented correctly, in which case we follow
                        // the "Content-Length"/"Transfer-Encoding" header
                        // instead (above).
                        None if !request_method.expects_body() || !status.includes_body() => {
                            BodyKind::Known { left: 0 }
                        }
                        // RFC 7230 section 3.3.3 point 7:
                        // > Otherwise, this is a response message without a
                        // > declared message body length, so the message body
                        // > length is determined by the number of octets
                        // > received prior to the server closing the
                        // > connection.
                        None => BodyKind::Unknown,
                    };
                    let body = Body { client: self, kind };
                    return Ok(Ok(Some(Response::new(version, status, headers, body))));
                }
                Ok(httparse::Status::Partial) => {
                    // Buffer doesn't include the entire response head, try
                    // reading more bytes (in the next iteration).
                    too_short = self.buf.len();
                    if too_short >= MAX_HEAD_SIZE {
                        return Ok(Err(ResponseError::HeadTooLarge));
                    }

                    continue;
                }
                Err(err) => return Ok(Err(ResponseError::from_httparse(err))),
            }
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
}

enum ResponseBodyLength {
    /// Body length is known.
    Known(usize),
    /// Body length is unknown and the body will be transfered using chunked
    /// encoding.
    Chunked,
    /// Body length is unknown, but the response is not chunked. Read until the
    /// connection is closed.
    ReadToEnd,
}

/// [`Future`] behind [`Client::connect`].
#[derive(Debug)]
pub struct Connect {
    connect: stream::Connect,
}

impl Future for Connect {
    type Output = io::Result<Client>;

    #[track_caller]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.connect).poll(ctx) {
            Poll::Ready(Ok(mut stream)) => {
                stream.set_nodelay(true)?;
                Poll::Ready(Ok(Client {
                    stream,
                    buf: Vec::with_capacity(BUF_SIZE),
                    parsed_bytes: 0,
                }))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
pub struct Body<'c> {
    client: &'c mut Client,
    kind: BodyKind,
}

#[derive(Debug)]
enum BodyKind {
    /// Known body length.
    Known {
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
    /// Body length is not known, read the body until the server closes the
    /// connection.
    Unknown,
}

impl<'c> Body<'c> {
    /*
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
            BodyKind::Known { left } => left == 0,
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
    */

    /*
    TODO: RFC 7230 section 3.3.3 point 5:
       [..] If the sender closes the connection or the recipient times out
       before the indicated number of octets are received, the recipient MUST
       consider the message to be incomplete and close the connection.
    */

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

            match &mut self.kind {
                // Read all the bytes from the body.
                BodyKind::Known { left: 0 } => return Ok(()),
                // Read all the bytes in the chunk, so need to read another
                // chunk.
                BodyKind::Chunked {
                    left_in_chunk,
                    read_complete,
                } if *left_in_chunk == 0 => {
                    if *read_complete {
                        return Ok(());
                    }

                    self.client.read_chunk(left_in_chunk, read_complete).await?;
                    // Copy read bytes again.
                    continue;
                }
                // Continue to reading below.
                BodyKind::Known { .. } | BodyKind::Chunked { .. } | BodyKind::Unknown => break,
            }
        }

        loop {
            // Limit the read until the end of the chunk/body.
            let chunk_len = match self.kind {
                BodyKind::Known { left } => Some(left),
                BodyKind::Chunked { left_in_chunk, .. } => Some(left_in_chunk),
                BodyKind::Unknown => None,
            };

            if let Some(chunk_len) = chunk_len {
                if chunk_len == 0 {
                    return Ok(());
                } else if total + chunk_len > limit {
                    return Err(io::Error::new(io::ErrorKind::Other, "body too large"));
                }
            }

            let capacity = chunk_len
                .unwrap_or_else(|| min(MIN_READ_SIZE, limit.saturating_sub(buf.capacity())));
            (&mut *buf).reserve(capacity);
            if let Some(chunk_len) = chunk_len {
                // FIXME: doesn't deal with chunked bodies.
                return self.client.stream.recv_n(&mut *buf, chunk_len).await;
            }
            let n = self.client.stream.recv(&mut *buf).await?;
            if n == 0 {
                return Ok(());
            }
            total += n;
            if total > limit {
                return Err(io::Error::new(io::ErrorKind::Other, "body too large"));
            }
        }
    }

    /// Returns the bytes currently in the buffer.
    ///
    /// This is limited to the bytes of this request/chunk, i.e. it doesn't
    /// contain the next request/chunk.
    fn buf_bytes(&self) -> &[u8] {
        let bytes = &self.client.buf[self.client.parsed_bytes..];
        match self.kind {
            BodyKind::Known { left }
            | BodyKind::Chunked {
                left_in_chunk: left,
                ..
            } if bytes.len() > left => &bytes[..left],
            _ => bytes,
        }
    }

    /// Mark `n` bytes are processed.
    fn processed(&mut self, n: usize) {
        // TODO: should this be `unsafe`? We don't do underflow checks...
        match &mut self.kind {
            BodyKind::Known { left } => *left -= n,
            BodyKind::Chunked { left_in_chunk, .. } => *left_in_chunk -= n,
            BodyKind::Unknown => {}
        }
        self.client.parsed_bytes += n;
    }
}

// FIXME: remove body from `Client` if it's dropped before it's fully read.

/// Error parsing HTTP response.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResponseError {
    /// Missing part of response.
    IncompleteResponse,
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
    /// Response contains both "Content-Length" and "Transfer-Encoding" headers.
    ///
    /// An attacker might attempt to "smuggle a request" ("HTTP Response
    /// Smuggling", Linhart et al., June 2005) or "split a response" ("Divide
    /// and Conquer - HTTP Response Splitting, Web Cache Poisoning Attacks, and
    /// Related Topics", Klein, March 2004). RFC 7230 (see section 3.3.3 point
    /// 3) says that this "ought to be handled as an error", and so we do.
    ContentLengthAndTransferEncoding,
    /// Invalid byte in new line.
    InvalidNewLine,
    /// Invalid byte in HTTP version.
    InvalidVersion,
    /// Invalid byte in status code.
    InvalidStatus,
    /// Chunk size is invalid.
    InvalidChunkSize,
}

impl ResponseError {
    /// Returns `true` if the connection should be closed based on the error
    /// (after sending a error response).
    #[allow(clippy::unused_self)]
    pub const fn should_close(self) -> bool {
        // Currently all errors are fatal for the connection.
        true
    }

    fn from_httparse(err: httparse::Error) -> ResponseError {
        use httparse::Error::*;
        match err {
            HeaderName => ResponseError::InvalidHeaderName,
            HeaderValue => ResponseError::InvalidHeaderValue,
            Token => unreachable!(),
            NewLine => ResponseError::InvalidNewLine,
            Version => ResponseError::InvalidVersion,
            TooManyHeaders => ResponseError::TooManyHeaders,
            Status => ResponseError::InvalidStatus,
        }
    }

    fn as_str(self) -> &'static str {
        use ResponseError::*;
        match self {
            IncompleteResponse => "incomplete response",
            HeadTooLarge => "response head too large",
            InvalidContentLength => "invalid response Content-Length header",
            DifferentContentLengths => "response has different Content-Length headers",
            InvalidHeaderName => "invalid response header name",
            InvalidHeaderValue => "invalid response header value",
            TooManyHeaders => "too many response headers",
            UnsupportedTransferEncoding => "response has unsupported Transfer-Encoding header",
            ContentLengthAndTransferEncoding => {
                "response contained both Content-Length and Transfer-Encoding headers"
            }
            InvalidNewLine => "invalid response syntax",
            InvalidVersion => "invalid HTTP response version",
            InvalidStatus => "invalid HTTP response status",
            InvalidChunkSize => "invalid response chunk size",
        }
    }
}

impl From<ResponseError> for io::Error {
    fn from(err: ResponseError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, err.as_str())
    }
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
