//! The module with the [`Initiator`] trait definition.

use std::io;

use system::ActorSystemRef;

// TODO: Implement Initiator for TcpListener:
// To create it it will take a `NewActor<Item = TcpStream>`. It calls `accept`
// and will create a new actor for the connection and add it to the system.

/// The `Initiator` is responsible for initiating events in the actor system.
///
/// This could be an TCP listener that will create a new event for each incoming
/// connection.
pub trait Initiator {
    /// Poll the `Initiator` for new events.
    fn poll(&mut self, system: &mut ActorSystemRef) -> io::Result<()>;
}
