//! The module with the supervisor related types.
//!
//! # Supervisor
//!
//! A supervisor is regular which can (also) receive a [`SupervisorMessage`].
//! This allows the actor to spawn other actors, making itself the supervisor of
//! the spawned actor, via [`ActorContext.spawn`].
//!
//! An actor can be a supervisor only, meaning the type of [message] will only
//! be `SupervisorMessage` and the supervisor itself won't do any computation.
//! This is recommended in most cases since its important for a supervisor to
//! respond quickly to failures of spawned actors.
//!
//! Alternatively actors can spawn other actors to split the computational load
//! while also doing computational work itself. In this case the message type
//! would be an enum with one of the variants being `SupervisorMessage`, which
//! requires the message to implement `From<SupervisorMessage>`. This allows the
//! actor to be a regular actor while also supervising other spawned actors.
//!
//! [`SupervisorMessage`]: enum.SupervisorMessage.html
//! [`ActorContext.spawn`]: ../actor/struct.ActorContext.html#method.spawn
//! [message]: ../actor/trait.NewActor.html#associatedtype.Message

/// Message type for supervisors.
///
/// This informs the supervisor of the stopping of a spawned actor.
#[derive(Debug)]
pub enum SupervisorMessage<E> {
    /// Actor stopped normally, i.e. it returned `Ok(())`.
    Done,
    /// Actor encountered an error, i.e. it returned an `Err(E)`.
    Error(E, ApplyStrategy),
}

// TODO: make `ApplyStrategy` `!Send` and `!Sync`.

/// A special reference to an actor that returned an error to apply a strategy
/// to.
///
/// This special reference allows the supervisor to deal with a failure from an
/// actor. For example it could restart it, or decide to stop it.
///
/// The default is to stop the actor, nothing has to be done for this it will
/// happen automatically once this is dropped.
#[derive(Debug)]
pub struct ApplyStrategy {
    strategy: Option<SupervisorStrategy>,
}

impl ApplyStrategy {
    /// Apply the strategy to the actor.
    pub fn apply(mut self, strategy: SupervisorStrategy) {
        self.strategy = Some(strategy);
        // The actual usage of the strategy is done in the drop implementation.
        // This way if this is dropped without setting a strategy the actor will
        // still be stopped.
    }
}

impl Drop for ApplyStrategy {
    fn drop(&mut self) {
        // TODO: actually do something with the strategy.
    }
}

/// The strategy to use when handling an actor's error.
///
/// The strategy comes in a number of flavours but basically two chooses have to
/// be made:
/// - To stop or to restart the actor?
/// - And to apply the strategy to just the erroneous actor, or the entire tree.
///
/// # Restarting or stopping?
///
/// Sometime just restarting an actor is the easiest way to deal with errors.
/// Starting the actor from a clean slate will allow it to continue processing.
/// However not in all cases is this possible.
///
/// # Apply to entire tree or only erroneous actor?
///
/// When the strategy is applied to the entire tree it means that all the
/// actors that the erroneous actor spawned will also be stopped/restarted.
///
/// For example we have actor A that spawns actors A1 and A2. If actor A returns
/// an error and the entire tree is stopped, it means that actors A1 and A2 will
/// also be stopped.
#[derive(Debug)]
pub enum SupervisorStrategy {
    /// Restart just the actor.
    RestartActor,
    /// Restart the entire actor tree.
    RestartTree,
    /// Stop just the actor.
    Stop,
    /// Stop the entire actor tree.
    StopTree,
}
