//! The module with the `Initiator` trait definition.

use std::io;

use mio_st::poll::Poller;

use crate::process::ProcessId;
use crate::system::ActorSystemRef;

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
pub trait Initiator: Sized + Send {
    /// Initialise the initiator.
    ///
    /// This will be called once the initiator is added to the actor system.
    /// Only after which `clone_threaded` is called.
    #[doc(hidden)]
    fn init(&mut self, poll: &mut Poller, pid: ProcessId) -> io::Result<()>;

    /// Create a clone of itself to be send to another thread.
    #[doc(hidden)]
    fn clone_threaded(&mut self) -> io::Result<Self>;

    /// Poll the `Initiator` for new events.
    ///
    /// It gets an [`ActorSystemRef`] so it can add actors to the system.
    ///
    /// [`ActorSystemRef`]: ../system/struct.ActorSystemRef.html
    #[doc(hidden)]
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}
