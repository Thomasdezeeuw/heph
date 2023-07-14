//! Runtime channel for use in communicating between the coordinator and a
//! worker thread.

use std::io::{self, Read, Write};
use std::sync::mpsc;

use mio::{unix, Interest, Registry, Token};

/// Data send across the channel to create a `mio::Event`.
const WAKE: &[u8] = b"WAKE";

/// Create a new communication channel.
pub(crate) fn new<T>() -> io::Result<(Sender<T>, Receiver<T>)> {
    let (p_send, p_recv) = unix::pipe::new()?;
    let (c_send, c_recv) = mpsc::channel();
    let sender = Sender {
        channel: c_send,
        pipe: p_send,
    };
    let receiver = Receiver {
        channel: c_recv,
        pipe: p_recv,
    };
    Ok((sender, receiver))
}

/// Sending end of the communication channel.
#[derive(Debug)]
pub(crate) struct Sender<T> {
    channel: mpsc::Sender<T>,
    pipe: unix::pipe::Sender,
}

impl<T> Sender<T> {
    /// Try to send a message onto the channel.
    pub(crate) fn try_send(&self, msg: T) -> io::Result<()> {
        self.channel
            .send(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "failed to send message"))?;

        // Generate an `mio::Event` for the receiving end.
        loop {
            match (&self.pipe).write(WAKE) {
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

    /// Register the sending end of the Unix pipe of this channel.
    pub(crate) fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()> {
        registry.register(&mut self.pipe, token, Interest::WRITABLE)
    }
}

/// Receiving end of the communication channel.
#[derive(Debug)]
pub(crate) struct Receiver<T> {
    channel: mpsc::Receiver<T>,
    pipe: unix::pipe::Receiver,
}

impl<T> Receiver<T> {
    /// Try to receive a message from the channel.
    pub(crate) fn try_recv(&mut self) -> io::Result<Option<T>> {
        if let Ok(msg) = self.channel.try_recv() {
            Ok(Some(msg))
        } else {
            // If the channel is empty this will likely be the last call in a
            // while, so we'll empty the pipe to ensure we'll get another
            // notification once the coordinator sends us another message.
            let mut buf = [0; 24]; // Fits 6 messages.
            loop {
                match self.pipe.read(&mut buf) {
                    Ok(n) if n < buf.len() => break,
                    // Didn't empty it.
                    Ok(..) => continue,
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err),
                }
            }
            // Try one last time in case the coordinator send a message in
            // between the time we last checked and we emptied the pipe above
            // (for which we won't get another event as we just emptied the
            // pipe).
            Ok(self.channel.try_recv().ok())
        }
    }

    /// Register the receiving end of the Unix pipe of this channel.
    pub(crate) fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()> {
        registry.register(&mut self.pipe, token, Interest::READABLE)
    }
}
