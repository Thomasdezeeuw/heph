//! Runtime channel for use in communicating between the coordinator and a
//! (sync) worker thread.

use std::io::{self, Read, Write};

use crossbeam_channel as crossbeam;
use mio::{Interest, Registry, Token};
use mio_pipe::{self, new_pipe};

/// A handle to a two-way communication channel, which can send messages `S` and
/// receive messages `R`.
#[derive(Debug)]
pub(crate) struct Handle<S, R> {
    /// Sending side.
    send_channel: crossbeam::Sender<S>,
    send_pipe: mio_pipe::Sender,
    /// Receiving side.
    recv_channel: crossbeam::Receiver<R>,
    recv_pipe: mio_pipe::Receiver,
}

/// Create a new two-way communication channel.
pub(crate) fn new<S, R>() -> io::Result<(Handle<S, R>, Handle<R, S>)> {
    let (c_send1, c_recv2) = crossbeam::unbounded();
    let (c_send2, c_recv1) = crossbeam::unbounded();

    let (p_send1, p_recv2) = new_pipe()?;
    let (p_send2, p_recv1) = new_pipe()?;

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
        let _ = self.send_channel.try_send(msg);

        // Generate an `mio::Event` for the receiving end.
        match self.send_pipe.write(DATA) {
            Ok(n) if n < DATA.len() => Err(io::ErrorKind::WriteZero.into()),
            Ok(_) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Try to receive a message from the channel.
    pub(super) fn try_recv(&mut self) -> io::Result<Option<R>> {
        let msg = self.recv_channel.try_recv().ok();
        if msg.is_some() {
            let mut buf = [0; DATA.len()];
            self.recv_pipe.read(&mut buf).map(|_| msg)
        } else {
            Ok(None)
        }
    }
}
