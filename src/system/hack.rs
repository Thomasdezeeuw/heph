//! Collection of hacks.

use std::convert::TryFrom;

use crate::actor_ref::ActorRef;

use super::{ActorSystemRef, Signal};

/// A hack to allow us to call `ActorSystem<!>.run`.
pub trait SetupFn: Send + Clone + 'static {
    type Error: Send;

    fn setup(self, system_ref: ActorSystemRef) -> Result<(), Self::Error>;
}

impl<F, E> SetupFn for F
where
    F: FnOnce(ActorSystemRef) -> Result<(), E> + Send + Clone + 'static,
    E: Send,
{
    type Error = E;

    fn setup(self, system_ref: ActorSystemRef) -> Result<(), Self::Error> {
        (self)(system_ref)
    }
}

impl SetupFn for ! {
    type Error = !;

    fn setup(self, _: ActorSystemRef) -> Result<(), Self::Error> {
        self
    }
}

/// Hack to allow `SyncWorker` to convert its actor reference to a reference
/// that receives signals if possible, or returns none.
pub(crate) trait IntoSignalActorRef {
    fn into(actor_ref: &Self) -> Option<ActorRef<Signal>>;
}

impl<M> IntoSignalActorRef for ActorRef<M> {
    default fn into(_: &Self) -> Option<ActorRef<Signal>> {
        None
    }
}

impl<M> IntoSignalActorRef for ActorRef<M>
where
    M: TryFrom<Signal, Error = Signal> + Into<Signal> + Sync + Send + 'static,
{
    fn into(actor_ref: &Self) -> Option<ActorRef<Signal>> {
        Some(actor_ref.clone().try_map())
    }
}
