//! Runtime channel for use in communicating between the coordinator and a
//! worker thread.

// TODO: remove `rt::channel` entirely and replace it with an `ActorRef` to the
// `worker::comm_actor`.

use std::future::poll_fn;
use std::io;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::Poll;

use a10::msg::{MsgListener, MsgToken};

const WAKE: u32 = 0;

/// Create a new communication channel.
///
/// The `sq` will be used to wake up the receiving end when sending.
pub(crate) fn new<T>(sq: a10::SubmissionQueue) -> io::Result<(Sender<T>, Receiver<T>)> {
    let (listener, token) = sq.clone().msg_listener()?;
    let (sender, receiver) = mpsc::channel();
    let sender = Sender { sender, sq, token };
    let receiver = Receiver { receiver, listener };
    Ok((sender, receiver))
}

/// Sending side of the communication channel.
#[derive(Debug)]
pub(crate) struct Sender<T> {
    sender: mpsc::Sender<T>,
    /// Receiver's submission queue and token used to wake it up.
    sq: a10::SubmissionQueue,
    token: MsgToken,
}

impl<T> Sender<T> {
    /// Send a message into the channel.
    pub(crate) fn send(&self, msg: T) -> io::Result<()> {
        self.sender
            .send(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "receiver closed channel"))?;
        self.sq.try_send_msg(self.token, WAKE)
    }
}

/// Receiving side of the communication channel.
#[derive(Debug)]
pub(crate) struct Receiver<T> {
    receiver: mpsc::Receiver<T>,
    listener: MsgListener,
}

impl<T> Receiver<T> {
    /// Receive a message from the channel.
    pub(crate) async fn recv(&mut self) -> Option<T> {
        loop {
            poll_fn(|ctx| match Pin::new(&mut self.listener).poll_next(ctx) {
                Poll::Ready(data) => {
                    debug_assert_eq!(data, Some(WAKE));
                    Poll::Ready(())
                }
                Poll::Pending => Poll::Pending,
            })
            .await;

            match self.receiver.try_recv() {
                Ok(msg) => return Some(msg),
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }
}
