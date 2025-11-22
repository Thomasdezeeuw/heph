//! Runtime channel for use in communicating between the coordinator and a
//! worker thread.

// TODO: remove `rt::channel` entirely and replace it with an `ActorRef` to the
// `worker::comm_actor`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{self, Poll};

use a10::msg;

use crate::wakers::no_ring_ctx;

const WAKE: msg::Message = u32::from_ne_bytes([b'W', b'A', b'K', b'E']); // 1162559831.

/// Create a new communication channel.
///
/// The `sq` will be used to wake up the receiving end when sending.
pub(crate) fn new<T>(sq: a10::SubmissionQueue) -> io::Result<(Sender<T>, Receiver<T>)> {
    let (listener, waker) = msg::listener(sq.clone())?;
    let (sender, receiver) = mpsc::channel();
    let sender = Sender { sender, waker };
    let receiver = Receiver { receiver, listener };
    Ok((sender, receiver))
}

/// Sending side of the communication channel.
#[derive(Debug)]
pub(crate) struct Sender<T> {
    #[allow(clippy::struct_field_names)]
    sender: mpsc::Sender<T>,
    waker: msg::Sender,
}

impl<T> Sender<T> {
    /// Send a message into the channel.
    pub(crate) fn send(&self, msg: T) -> io::Result<()> {
        self.sender
            .send(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "receiver closed channel"))?;
        self.waker.try_send(WAKE)
    }
}

/// Receiving side of the communication channel.
#[derive(Debug)]
pub(crate) struct Receiver<T> {
    receiver: mpsc::Receiver<T>,
    listener: msg::Listener,
}

impl<T> Receiver<T> {
    /// Receive a message from the channel.
    pub(crate) fn recv<'r>(&'r mut self) -> Receive<'r, T> {
        Receive { receiver: self }
    }
}

/// [`Future`] behind [`Receiver::recv`].
pub(crate) struct Receive<'r, T> {
    receiver: &'r mut Receiver<T>,
}

impl<'r, T> Future for Receive<'r, T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let receiver = &mut *self.receiver;
        loop {
            // Check if we have a message first.
            if let Ok(msg) = receiver.receiver.try_recv() {
                return Poll::Ready(Some(msg));
            }

            // If not wait until we get a signal that another message is
            // available.
            no_ring_ctx!(ctx);
            match Pin::new(&mut receiver.listener).poll_next(ctx) {
                Poll::Ready(data) => {
                    debug_assert_eq!(data, Some(WAKE));
                    continue;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
