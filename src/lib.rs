//! TODO: docs.

#![feature(arbitrary_self_types,
           const_fn,
           futures_api,
           never_type,
           non_exhaustive,
           pin,
           read_initializer,
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

#[macro_use]
extern crate log;
extern crate mio_st;
extern crate num_cpus;
extern crate slab;

#[cfg(all(test, feature = "test"))]
extern crate env_logger;

pub mod actor;
pub mod initiator;
pub mod io;
pub mod net;
pub mod supervisor;
pub mod system;

mod process;
mod scheduler;
mod util;

/// The actor prelude. All useful traits and types in single module.
///
/// ```
/// use actor::prelude::*;
/// ```
pub mod prelude {
    pub use actor::{Actor, NewActor};
    pub use supervisor::{Supervisor, RestartStrategy};
    pub use system::{ActorSystem, ActorSystemBuilder, ActorOptions, ActorRef};
}
