extern crate futures_core;

pub mod actor;

/// The actor prelude. All useful traits and types in single module.
///
/// ```
/// use actor::prelude::*;
/// ```
pub mod prelude {
    pub use actor::{Actor, NewActor, actor_factory, reusable_actor_factory};
}
