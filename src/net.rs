//! Network related types.

use std::io::{self, ErrorKind, Initializer, Read, Write};
use std::net::SocketAddr;

use mio_st::event::Ready;
use mio_st::net::TcpListener as MioTcpListener;
use mio_st::net::TcpStream as MioTcpStream;
use mio_st::poll::{Poller, PollOption};

use actor::{Actor, NewActor};
use initiator::Initiator;
use process::ProcessId;
use system::{ActorSystemRef, ActorOptions};

/// A TCP listener that implements the [`Initiator`] trait.
///
/// For each accepted connection a new actor will be created by using the
/// [`NewActor`] trait. The provided `options` to the `bind` method will be used
/// in adding the newly created actor to the actor system.
///
/// [`Initiator`]: ../initiator/trait.Initiator.html
/// [`NewActor`]: ../actor/trait.NewActor.html
#[derive(Debug)]
pub struct TcpListener<N> {
    /// The underlying TCP listener, backed by mio.
    listener: MioTcpListener,
    /// The `NewActor` implement to create a new actor for each incoming
    /// connection.
    new_actor: N,
    /// Options used to add the actor to the actor system.
    options: ActorOptions,
}

// TODO: remove the static lifetime from `A`, it also needs to be removed from
// the ActorSystem.add_actor_setup.

impl<N, A> TcpListener<N>
    where N: NewActor<Item = (TcpStream, SocketAddr), Actor = A>,
          A: Actor + 'static,
{
    /// Bind a new TCP listener to the provided `address`.
    pub fn bind(address: SocketAddr, new_actor: N, options: ActorOptions) -> io::Result<TcpListener<N>> {
        Ok(TcpListener {
            listener: MioTcpListener::bind(address)?,
            new_actor,
            options,
        })
    }
}

impl<N, A> Initiator for TcpListener<N>
    where N: NewActor<Item = (TcpStream, SocketAddr), Actor = A>,
          A: Actor + 'static,
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

            let system_ref_clone = system_ref.clone();
            let _ = system_ref.add_actor_setup(self.options.clone(), |pid, poller| {
                poller.register(&mut stream, pid.into(),
                    Ready::READABLE | Ready::WRITABLE | Ready::ERROR | Ready::HUP,
                    PollOption::Edge)?;

                // Wrap the raw stream with our wrapper.
                let stream = TcpStream {
                    inner: stream,
                    system_ref: system_ref_clone,
                };

                // Create our actor and add it the system.
                Ok(self.new_actor.new((stream, addr)))
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

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }

    unsafe fn initializer(&self) -> Initializer {
        self.inner.initializer()
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/*

TODO: enable this once AsyncRead and AsyncWrite are available in futures v0.3.

/// A macro to try an I/O function.
///
/// It runs the `$op` operation, expecting an `io::Result` on which it pattern
/// matches to deal with the error accordingly. I.e. trying again, by using the
/// `$retry` operation, if the returned error is of kind interrupted or
/// returning `Async::Pending` if the is error is of kind would block.
///
/// The second variant allows for a special action to be taken in case of an OK
/// result.
macro_rules! try_io {
    ($op:expr, $retry:expr) => (
        match $op {
            Ok(ok) => Ok(Async::Ready(ok)),
            Err(ref err) if would_block(err) => Ok(Async::Pending),
            Err(ref err) if interrupted(err) => $retry,
            Err(err) => Err(err),
        }
    );
    ($op:expr, $retry:expr, $ok_op:block) => (
        match $op {
            Ok(_) => $ok_op,
            Err(ref err) if would_block(err) => Ok(Async::Pending),
            Err(ref err) if interrupted(err) => $retry,
            Err(err) => Err(err),
        }
    );
}

impl AsyncRead for TcpStream {
    // TODO: add initializer.

    fn poll_read(&mut self, ctx: &mut Context, buf: &mut [u8]) -> Poll<usize, io::Error> {
        try_io!(self.inner.read(buf), self.poll_read(ctx, buf))
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(&mut self, ctx: &mut Context, buf: &[u8]) -> Poll<usize, io::Error> {
        try_io!(self.inner.write(buf), self.poll_write(ctx, buf))
    }

    fn poll_flush(&mut self, ctx: &mut Context) -> Poll<(), io::Error> {
        try_io!(self.inner.flush(), self.poll_flush(ctx))
    }

    fn poll_close(&mut self, ctx: &mut Context) -> Poll<(), io::Error> {
        // First flush the connection, next shut it down.
        try_io!(self.poll_flush(ctx), self.poll_close(ctx), {
            try_io!(self.inner.shutdown(Shutdown::Both), self.poll_close(ctx))
        })
    }
}
*/

impl Drop for TcpStream {
    fn drop(&mut self) {
        if let Err(err) = self.system_ref.poller_deregister(&mut self.inner) {
            error!("error deregistering TcpStream from ActorSystem: {}", err);
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
