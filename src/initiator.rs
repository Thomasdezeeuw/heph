//! The module with the `Initiator` trait definition.

use std::io;

use mio_st::poll::Poller;

use process::ProcessId;
use system::ActorSystemRef;

// TODO: maybe let Initator return a specialised error, other then `io::Error`?
// E.g. `RuntimeError`.

/// The `Initiator` is responsible for initiating events in the actor system.
///
/// Implementations of this trait can be found [below]. This includes a TCP
/// listener that will create a new actor for each incoming connection.
///
/// [below]: #implementors
///
/// # Notes
///
/// This trait is private and can only be implemented by internal types. It's
/// only public for documentation purposes.
pub trait Initiator {
    /// Initialise the initiator.
    ///
    /// This will be called once when the actor system starts running, by
    /// calling the [`ActorSystem.run`] method.
    ///
    /// [`ActorSystem.run`]: ../system/struct.ActorSystem.html#method.run
    #[doc(hidden)]
    fn init(&mut self, poll: &mut Poller, pid: ProcessId) -> io::Result<()>;

    /// Poll the `Initiator` for new events.
    ///
    /// It gets an [`ActorSystemRef`] so it can add actors to the system.
    ///
    /// [`ActorSystemRef`]: ../system/struct.ActorSystemRef.html
    #[doc(hidden)]
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}
