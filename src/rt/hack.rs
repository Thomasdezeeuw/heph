//! Collection of hacks.

use std::convert::TryFrom;

use crate::actor_ref::ActorRef;
use crate::rt::{RuntimeRef, Signal};

/// A hack to allow us to call `Runtime<!>.run`.
pub trait SetupFn: Send + Clone + 'static {
    type Error: Send;

    fn setup(self, runtime_ref: RuntimeRef) -> Result<(), Self::Error>;
}

impl<F, E> SetupFn for F
where
    F: FnOnce(RuntimeRef) -> Result<(), E> + Send + Clone + 'static,
    E: Send,
{
    type Error = E;

    fn setup(self, runtime_ref: RuntimeRef) -> Result<(), Self::Error> {
        (self)(runtime_ref)
    }
}

impl SetupFn for ! {
    type Error = !;

    fn setup(self, _: RuntimeRef) -> Result<(), Self::Error> {
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
