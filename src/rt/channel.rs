//! Runtime channel for use in communicating between the coordinator and a
//! (sync) worker thread.

use std::io::{self, Read, Write};

use crossbeam_channel as crossbeam;
use log::trace;
use mio::unix::pipe;
use mio::{Interest, Registry, Token};

/// A handle to a two-way communication channel, which can send messages `S` and
/// receive messages `R`.
#[derive(Debug)]
pub(crate) struct Handle<S, R> {
    /// Sending side.
    send_channel: crossbeam::Sender<S>,
    send_pipe: pipe::Sender,
    /// Receiving side.
    recv_channel: crossbeam::Receiver<R>,
    recv_pipe: pipe::Receiver,
}

/// Create a new two-way communication channel.
pub(crate) fn new<S, R>() -> io::Result<(Handle<S, R>, Handle<R, S>)> {
    let (c_send1, c_recv2) = crossbeam::unbounded();
    let (c_send2, c_recv1) = crossbeam::unbounded();

    let (p_send1, p_recv2) = pipe::new()?;
    let (p_send2, p_recv1) = pipe::new()?;

    let handle1 = Handle {
        send_channel: c_send1,
        send_pipe: p_send1,
        recv_channel: c_recv1,
        recv_pipe: p_recv1,
    };

    let handle2 = Handle {
        send_channel: c_send2,
        send_pipe: p_send2,
        recv_channel: c_recv2,
        recv_pipe: p_recv2,
    };

    Ok((handle1, handle2))
}

/// Data send across the channel to create a `mio::Event`.
const DATA: &[u8] = b"DATA";

impl<S, R> Handle<S, R> {
    /// Register both ends of the Unix pipe of this channel.
    pub(super) fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()> {
        registry
            .register(&mut self.send_pipe, token, Interest::WRITABLE)
            .and_then(|()| registry.register(&mut self.recv_pipe, token, Interest::READABLE))
    }

    /// Try to send a message onto the channel.
    pub(super) fn try_send(&mut self, msg: S) -> io::Result<()> {
        self.send_channel
            .try_send(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "failed to send message"))?;

        // Generate an `mio::Event` for the receiving end.
        loop {
            trace!("notifying worker-coordinator channel of new message");
            match self.send_pipe.write(DATA) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "failed to send into channel pipe",
                    ))
                }
                Ok(..) => return Ok(()),
                // Can't do too much here.
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }

    /// Try to receive a message from the channel.
    pub(super) fn try_recv(&mut self) -> io::Result<Option<R>> {
        match self.recv_channel.try_recv().ok() {
            Some(msg) => Ok(Some(msg)),
            None => {
                // If the channel is empty this will likely be the last call in
                // a while, so we'll empty the pipe to ensure we'll get another
                // notification once the coordinator sends us another message.
                let mut buf = [0; 5 * DATA.len()];
                loop {
                    trace!("emptying worker-coordinator channel pipe");
                    match self.recv_pipe.read(&mut buf) {
                        Ok(n) if n < buf.len() => break,
                        // Didn't empty it.
                        Ok(..) => continue,
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                        Err(err) => return Err(err),
                    }
                }
                // Try one last time in case the coordinator send a message
                // in between the time we last checked and we emptied the pipe
                // above.
                Ok(self.recv_channel.try_recv().ok())
            }
        }
    }

    /// Checks if the other side is alive.
    pub(super) fn is_alive(&mut self) -> bool {
        match self.send_pipe.write(&[]) {
            Ok(..) => true,
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => true,
            Err(..) => false,
        }
    }
}
