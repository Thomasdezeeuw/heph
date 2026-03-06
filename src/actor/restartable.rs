use crate::actor::{self, NewActor};

/// Actors that can be stopped and restarted.
pub trait RestartableActor: NewActor {
    /// State from which the actor can be restarted.
    ///
    /// # Notes
    ///
    /// Any not received messages (not removed from the inbox) should not be
    /// part of the state. Any messages that are received should be processed or
    /// be part of the state (to be processed on restart).
    ///
    /// Implementations that use this trait to store the state to stable
    /// storage, e.g. to disk, or to transfer the actor to another process or
    /// even machine will have additional requirements on the `State` type.
    type State;

    /// Return the state of a stopped actor.
    fn stop(&mut self, actor: Self::Actor) -> Self::State;

    /// Restart an actor from a previously stopped state.
    ///
    /// Similar to [`NewActor::new`], but instead of starting argument a
    /// previous state is passed.
    fn restart(
        &mut self,
        ctx: actor::Context<Self::Message>,
        state: Self::State,
    ) -> Result<Self::Actor, Self::Error>;
}
