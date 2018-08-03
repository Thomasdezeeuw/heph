//! TODO: docs.

#![feature(arbitrary_self_types,
           const_fn,
           futures_api,
           never_type,
           non_exhaustive,
           pin,
           read_initializer,
           rust_2018_preview,
)]

#![warn(anonymous_parameters,
        bare_trait_objects,
        missing_debug_implementations,
        missing_docs,
        trivial_casts,
        trivial_numeric_casts,
        unused_extern_crates,
        unused_import_braces,
        unused_qualifications,
        unused_results,
        variant_size_differences,
)]

#[cfg(all(test, feature = "test"))]
extern crate env_logger;

pub mod actor;
pub mod actor_ref;
pub mod error;
pub mod initiator;
pub mod net;
//pub mod supervisor;
pub mod system;
pub mod timer;

mod mailbox;
mod process;
mod scheduler;
mod util;
mod waker;

/// The actor prelude. All useful traits and types in single module.
///
/// ```
/// use heph::prelude::*;
/// ```
pub mod prelude {
    pub use crate::actor::{Actor, NewActor};
    pub use crate::actor_ref::{ActorRef, LocalActorRef};
    //pub use crate::supervisor::{Supervisor, RestartStrategy};
    pub use crate::system::{ActorSystem, ActorSystemBuilder, ActorOptions};
}
