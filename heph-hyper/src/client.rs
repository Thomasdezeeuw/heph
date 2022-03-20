use std::future::{Future, Ready};
use std::io::{self, IoSlice};
use std::mem::MaybeUninit;
use std::net::{Shutdown, SocketAddr};
use std::pin::Pin;
use std::task::{self, Poll};

use heph::bytes::Bytes;
use heph::net::tcp::stream::{Connect, TcpStream};
use heph::rt;
use heph::rt::RuntimeRef;
use hyper::client::connect::{self, Connected};
use hyper::service::Service;
use hyper::{Request, Uri};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub type Client<B> = hyper::client::Client<HttpConnector, B>;

pub struct HttpConnector {
    rt: RuntimeRef,
}

impl HttpConnector {
    fn new(rt: RuntimeRef) -> HttpConnector {
        HttpConnector { rt }
    }
}

impl Service<Uri> for HttpConnector {
    type Response = Connection;

    type Error = io::Error;

    type Future = HttpConnect;

    fn poll_ready(&mut self, ctx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        // FIXME: return an error, don't unwrap.
        let host = uri.host().unwrap();
        let port = match uri.port_u16() {
            Some(port) => port,
            None => match uri.scheme_str() {
                Some("http") => 80,
                Some("https") => 433,
                // FIXME: return an error.
                _ => todo!("can't get port"),
            },
        };

        // FIXME: this is blocking!
        // FIXME: don't unwrap.
        let addresses = std::net::ToSocketAddrs::to_socket_addrs(&(host, port)).unwrap();
        // FIXME: don't unwrap return an error.
        let address = addresses.next().unwrap();
        let ctx = todo!("get actor context out of nowhere!");
        let connect = TcpStream::connect(&mut ctx, address).unwrap();
        HttpConnect { connect }
    }
}

pub struct HttpConnect {
    connect: Connect,
}

impl Future for HttpConnect {
    type Output = io::Result<Connection>;

    #[track_caller]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.connect)
            .poll(ctx)
            .map_ok(|stream| Connection { stream })
    }
}

/// HTTP [`Client`] connection.
pub struct Connection {
    stream: TcpStream,
}

impl connect::Connection for Connection {
    fn connected(&self) -> Connected {
        let connected = Connected::new();
        if let Ok(remote_addr) = self.stream.peer_addr() {
            connected.extra(HttpInfo { remote_addr })
        } else {
            connected
        }
    }
}

/// Additional information provided by [`Connection`].
#[derive(Clone, Debug)]
pub struct HttpInfo {
    remote_addr: SocketAddr,
}

impl HttpInfo {
    /// Returns the remote address.
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match self.stream.try_recv(ReadBufWrapper(buf)) {
                Ok(n) => break Poll::Ready(Ok(())),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// Wrapper around [`ReadBuf`] that implements [`Bytes`].
struct ReadBufWrapper<'a, 'b>(&'a mut ReadBuf<'b>);

impl<'a, 'b> Bytes for ReadBufWrapper<'a, 'b> {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        // SAFETY: not de-initializing bytes.
        unsafe { self.0.unfilled_mut() }
    }

    fn spare_capacity(&self) -> usize {
        self.0.remaining()
    }

    fn has_spare_capacity(&self) -> bool {
        self.0.remaining() == 0
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.0.assume_init(n);
        self.0.advance(n);
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match self.stream.try_send(buf) {
                Ok(n) => break Poll::Ready(Ok(n)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        loop {
            match self.stream.try_send_vectored(bufs) {
                Ok(n) => break Poll::Ready(Ok(n)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.stream.shutdown(Shutdown::Both))
    }
}

#[test]
fn test() {
    use hyper::client::connect::Connect;
    fn is_connect<T: Connect>() {}
    is_connect::<HttpConnector>();
}
