//! Network related types.

use std::io::{self, ErrorKind, Read, Write};
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use futures_io::{AsyncRead, AsyncWrite, Initializer};

use mio_st::event::Ready;
use mio_st::net::{TcpListener as MioTcpListener, TcpStream as MioTcpStream};
use mio_st::poll::{Poller, PollOption};

use crate::actor::{Actor, ActorContext};
use crate::initiator::Initiator;
use crate::process::ProcessId;
use crate::system::{ActorSystemRef, ActorOptions};

/// A TCP listener that implements the [`Initiator`] trait.
///
/// [`Initiator`]: ../initiator/trait.Initiator.html
#[derive(Debug)]
pub struct TcpListener<A> {
    /// The underlying TCP listener, backed by mio.
    listener: MioTcpListener,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
    actor: PhantomData<A>,
}

// TODO: remove the static lifetime from `A`, it also needs to be removed from
// the ActorSystem.add_actor_setup.

impl<A> TcpListener<A>
    where A: Actor<Item = (TcpStream, SocketAddr)> + 'static,
{
    /// Bind a new TCP listener to the provided `address`.
    ///
    /// For each accepted connection a new actor will be created by using the
    /// [`Actor::new`] method with an `TcpStream` and `SocketAddr`. The provided
    /// `options` will be used in adding the newly created actor to the actor
    /// system.
    ///
    /// [`Actor::new`]: ../actor/trait.Actor.html#tymethod.new
    pub fn bind(address: SocketAddr, options: ActorOptions) -> io::Result<TcpListener<A>> {
        Ok(TcpListener {
            listener: MioTcpListener::bind(address)?,
            options,
            actor: PhantomData,
        })
    }
}

impl<A> Initiator for TcpListener<A>
    where A: Actor<Item = (TcpStream, SocketAddr)> + 'static,
{
    fn init(&mut self, poller: &mut Poller, pid: ProcessId) -> io::Result<()> {
        poller.register(&mut self.listener, pid.into(),
            Ready::READABLE | Ready::ERROR, PollOption::Edge)
    }

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
            system_ref.add_actor_setup(self.options.clone(), |pid, poller| {
                poller.register(&mut stream, pid.into(),
                    Ready::READABLE | Ready::WRITABLE | Ready::ERROR | Ready::HUP,
                    PollOption::Edge)?;

                // Wrap the raw stream with our wrapper.
                let stream = TcpStream {
                    inner: stream,
                    system_ref: system_ref_clone,
                };

                // Create our actor and add it the system.
                Ok(A::new((stream, addr)))
            })?;
        }
    }
}

/// TODO: docs.
#[derive(Debug)]
pub struct TcpStream {
    /// Underlying TCP connection.
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
    pub fn connect(ctx: &mut ActorContext, address: SocketAddr) -> io::Result<TcpStream> {
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
    ($op:expr) => (
        loop {
            match $op {
                Ok(ok) => return Poll::Ready(Ok(ok)),
                Err(ref err) if would_block(err) => return Poll::Pending,
                Err(ref err) if interrupted(err) => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    );
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
