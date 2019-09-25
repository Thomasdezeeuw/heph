//! Module containing actor reference error types.

use std::error::Error;
use std::fmt;

/// Error returned when sending a message fails.
///
/// The reason why the sending of the message failed is unspecified.
#[derive(Copy, Clone, Debug)]
pub struct SendError;

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl Error for SendError {
    fn description(&self) -> &str {
        "unable to send message"
    }
}

/// Error returned when the actor is shutdown.
///
/// This is only possible to detect on local references.
#[derive(Copy, Clone, Debug)]
pub struct ActorShutdown;

impl fmt::Display for ActorShutdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl Error for ActorShutdown {
    fn description(&self) -> &str {
        "actor shutdown"
    }
}
