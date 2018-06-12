//! Module containing the errors for the `ActorSystem`.

/// Error when sending messages goes wrong.
#[derive(Debug, Eq, PartialEq)]
pub struct SendError<M> {
    /// The message that failed to send.
    pub message: M,
    /// The reason why the sending failed.
    pub reason: SendErrorReason,
}

// TODO: impl Error for `SendError`.

/// The reason why sending a message failed.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum SendErrorReason {
    /// The actor, to which the message was meant to be sent, is shutdown.
    ActorShutdown,
    /// The system is shutting down.
    ///
    /// When the system is shutting down no more message sending is allowed.
    SystemShutdown,
}

/// Error returned by running an `ActorSystem`.
#[derive(Debug)]
pub struct RuntimeError {
}
