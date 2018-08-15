//! Network related types.

use std::io::{self, ErrorKind, Read, Write};
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_io::{AsyncRead, AsyncWrite, Initializer};
use log::{debug, error, log};

use mio_st::event::Ready;
use mio_st::net::{TcpListener as MioTcpListener, TcpStream as MioTcpStream};
use mio_st::poll::{PollOption, Poller};

use crate::actor::{ActorContext, NewActor};
use crate::initiator::Initiator;
use crate::process::ProcessId;
use crate::system::{ActorOptions, ActorSystemRef};

/// A TCP listener that implements the [`Initiator`] trait.
///
/// This listener will accept TCP connections and for each incoming connection
/// create an actor, via the [`NewActor`] trait.
///
/// [`Initiator`]: ../initiator/trait.Initiator.html
/// [`NewActor`]: ../actor/trait.NewActor.html
///
/// # Example
///
/// The following example is a TCP server that writes "Hello World" to the
/// connection.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
/// use std::net::SocketAddr;
///
/// use futures_util::AsyncWriteExt;
///
/// use heph::actor::{actor_factory, ActorContext};
/// use heph::net::{TcpListener, TcpStream};
/// use heph::system::{ActorOptions, ActorSystem, InitiatorOptions};
///
/// async fn conn_actor(_ctx: ActorContext<!>, (mut stream, address): (TcpStream, SocketAddr)) -> io::Result<()> {
///     await!(stream.write_all(b"Hello World"))
/// }
///
/// // The address to listen on.
/// let address = "127.0.0.1:7890".parse().unwrap();
///
/// // Create our TCP listener. We'll use the default actor options.
/// let new_actor = actor_factory(conn_actor);
/// let listener = TcpListener::bind(address, new_actor, ActorOptions::default())
///     .expect("unable to bind TCP listener");
///
/// // We create our actor system.
/// ActorSystem::new()
///     // We add our TCP listener, using the default options.
///     .with_initiator(listener, InitiatorOptions::default())
///     # ; // We don't actually want to run this.
/// ```
#[derive(Debug)]
pub struct TcpListener<N> {
    /// The underlying TCP listener, backed by mio.
    listener: MioTcpListener,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
    /// NewActor used to create an actor for each connection.
    new_actor: N,
}

impl<N> TcpListener<N>
    where N: NewActor<StartItem = (TcpStream, SocketAddr)> + 'static + Clone + Send,
{
    /// Bind a new TCP listener to the provided `address`.
    ///
    /// For each accepted connection a new actor will be created by using the
    /// [`NewActor::new`] method with a `TcpStream` and `SocketAddr`. The
    /// provided `options` will be used in adding the newly created actor to the
    /// actor system.
    ///
    /// [`NewActor::new`]: ../actor/trait.NewActor.html#tymethod.new
    pub fn bind(address: SocketAddr, new_actor: N, options: ActorOptions) -> io::Result<TcpListener<N>> {
        Ok(TcpListener {
            listener: MioTcpListener::bind(address)?,
            options,
            new_actor,
        })
    }
}

impl<N> Initiator for TcpListener<N>
    where N: NewActor<StartItem = (TcpStream, SocketAddr)> + 'static + Clone + Send,
{
    #[doc(hidden)]
    fn clone_threaded(&self) -> io::Result<Self> {
        Ok(TcpListener {
            listener: self.listener.try_clone()?,
            options: self.options.clone(),
            new_actor: self.new_actor.clone(),
        })
    }

    #[doc(hidden)]
    fn init(&mut self, poller: &mut Poller, pid: ProcessId) -> io::Result<()> {
        poller.register(&mut self.listener, pid.into(),
            Ready::READABLE | Ready::ERROR, PollOption::Edge)
    }

    #[doc(hidden)]
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()> {
        loop {
            let (mut stream, addr) = match self.listener.accept() {
                Ok(ok) => ok,
                Err(ref err) if would_block(err) => return Ok(()),
                Err(ref err) if interrupted(err) => continue, // Try again.
                Err(err) => return Err(err),
            };
            debug!("accepted connection from: {}", addr);

            let system_ref_clone = system_ref.clone();
            system_ref.add_actor_setup(self.options.clone(), |ctx, pid, poller| {
                poller.register(&mut stream, pid.into(),
                    Ready::READABLE | Ready::WRITABLE | Ready::ERROR | Ready::HUP,
                    PollOption::Edge)?;

                // Wrap the raw stream with our wrapper.
                let stream = TcpStream {
                    inner: stream,
                    system_ref: system_ref_clone,
                };

                // Create our actor and add it the system.
                Ok(self.new_actor.new(ctx, (stream, addr)))
            })?;
        }
    }
}

/// A non-blocking TCP stream between a local socket and a remote socket.
#[derive(Debug)]
pub struct TcpStream {
    /// Underlying TCP connection, backed by mio.
    inner: MioTcpStream,
    /// A reference to the actor system in which this connection is located,
    /// used to deregister itself when dropped.
    system_ref: ActorSystemRef,
}

impl TcpStream {
    /// Create a new TCP stream.
    ///
    /// Create a new TCP stream and issue a non-blocking connect to the
    /// specified `address`.
    pub fn connect<M>(ctx: &mut ActorContext<M>, address: SocketAddr) -> io::Result<TcpStream> {
        let mut stream = MioTcpStream::connect(address)?;
        let mut system_ref = ctx.system_ref().clone();
        system_ref.poller_register(&mut stream, ctx.pid().into(),
            Ready::READABLE | Ready::WRITABLE | Ready::ERROR | Ready::HUP,
            PollOption::Edge)?;
        Ok(TcpStream {
            inner: stream,
            system_ref,
        })
    }
}

/// A macro to try an I/O function.
macro_rules! try_io {
    ($op:expr) => {
        loop {
            match $op {
                Ok(ok) => return Poll::Ready(Ok(ok)),
                Err(ref err) if would_block(err) => return Poll::Pending,
                Err(ref err) if interrupted(err) => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    };
}

impl AsyncRead for TcpStream {
    unsafe fn initializer(&self) -> Initializer {
        Initializer::nop()
    }

    fn poll_read(&mut self, _ctx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        try_io!(self.inner.read(buf))
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(&mut self, _ctx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        try_io!(self.inner.write(buf))
    }

    fn poll_flush(&mut self, _ctx: &mut Context) -> Poll<io::Result<()>> {
        try_io!(self.inner.flush())
    }

    fn poll_close(&mut self, ctx: &mut Context) -> Poll<io::Result<()>> {
        self.poll_flush(ctx)
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        if let Err(err) = self.system_ref.poller_deregister(&mut self.inner) {
            error!("error deregistering TCP connection from actor system: {}", err);
        }
    }
}

/// Whether or not the error is a would block error.
fn would_block(err: &io::Error) -> bool {
    err.kind() == ErrorKind::WouldBlock
}

/// Whether or not the error is an interrupted error.
fn interrupted(err: &io::Error) -> bool {
    err.kind() == ErrorKind::Interrupted
}
