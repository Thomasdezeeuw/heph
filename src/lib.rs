extern crate futures_core;

pub mod actor;

/// The actor prelude. All useful traits and structs in single module.
///
/// ```
/// use actor::prelude::*;
/// ```
pub mod prelude {
    pub use actor::{Actor, NewActor, ActorFactory, ReusableActorFactory};
}
