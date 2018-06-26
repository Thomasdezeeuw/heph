//! The module with the `Initiator` trait definition.

use std::io;

use mio_st::poll::Poll;

use system::ActorSystemRef;
use system::ProcessId;

// TODO: maybe let Initator return a specialised error, other then `io::Error`?
// E.g. `RuntimeError`.

/// The `Initiator` is responsible for initiating events in the actor system.
///
/// This could be an TCP listener that will create a new event for each incoming
/// connection.
pub trait Initiator {
    /// Initialise the initiator.
    ///
    /// This will be called once when the actor system start running, by calling
    /// the [`ActorSystem.run`] method.
    ///
    /// [`ActorSystem.run`]: ../system/struct.ActorSystem.html#method.run
    fn init(&mut self, system_ref: &mut Poll, pid: ProcessId) -> io::Result<()>;

    /// Poll the `Initiator` for new events.
    ///
    /// It gets an [`ActorSystemRef`] so it can actors to the system.
    ///
    /// [`ActorSystemRef`]: ../system/struct.ActorSystemRef.html
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}
