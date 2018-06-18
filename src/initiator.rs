//! The module with the `Initiator` trait definition.

use std::io;

use system::ActorSystemRef;

// TODO: Implement Initiator for TcpListener:
// To create it it will take a `NewActor<Item = TcpStream>`. It calls `accept`
// and will create a new actor for the connection and add it to the system.
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
    fn init(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;

    /// Poll the `Initiator` for new events.
    ///
    /// It gets an [`ActorSystemRef`] so it can actors to the system.
    ///
    /// [`ActorSystemRef`]: ../system/struct.ActorSystemRef.html
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}

/// A helper struct to allow the actor system to be run without any initiators.
///
/// The `Initiator` implementation is effectively a no-op.
///
/// # Examples
///
/// Running the [`ActorSystem`] without initiators.
///
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
///
/// ```
/// use actor::initiator::NoInitiator;
/// use actor::system::ActorSystemBuilder;
///
/// let mut actor_system = ActorSystemBuilder::default().build()
///     .expect("failed to build actor system");
///
/// // Add actors etc.
///
/// actor_system.run::<NoInitiator>(&mut [])
///     .expect("failed to run actor system");
/// ```
#[derive(Debug)]
pub struct NoInitiator;

impl Initiator for NoInitiator {
    fn init(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        Ok(())
    }

    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        Ok(())
    }
}
