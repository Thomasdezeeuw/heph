//! The module with the `Initiator` trait definition.

use std::io;

use mio_st::poll::Poller;

use crate::process::ProcessId;
use crate::system::ActorSystemRef;

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
    /// Create a clone of itself to be send to another thread.
    #[doc(hidden)]
    fn clone_threaded(&self) -> io::Result<Self>;

    /// Initialise the initiator.
    ///
    /// This will be called after `clone_threaded` is called, on the thread the
    /// initiator needs to run on.
    #[doc(hidden)]
    fn init(&mut self, poll: &mut Poller, pid: ProcessId) -> io::Result<()>;

    /// Poll the `Initiator` for new events.
    #[doc(hidden)]
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}

/// This is used by `ActorSystemBuilder` when no initiators are used.
#[doc(hidden)]
impl Initiator for ! {
    fn clone_threaded(&self) -> io::Result<Self> {
        *self
    }

    fn init(&mut self, _poll: &mut Poller, _pid: ProcessId) -> io::Result<()> {
        *self
    }

    fn poll(&mut self, _system_ref: &mut ActorSystemRef) -> io::Result<()> {
        *self
    }
}
