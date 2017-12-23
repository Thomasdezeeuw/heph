// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

#![feature(associated_type_defaults)]

extern crate fnv;
extern crate futures;
#[macro_use]
extern crate log;
extern crate mio;
extern crate tchannel;

pub mod actor;
pub mod listener;
pub mod supervisor;
pub mod system;

/// The actor prelude. All useful traits and structs in single module.
///
/// ```
/// use actor::prelude::*;
/// ```
pub mod prelude {
    pub use actor::{Actor, NewActor, ActorFactory, ReusableActorFactory};
    pub use system::{ActorSystem, ActorRef};
}
