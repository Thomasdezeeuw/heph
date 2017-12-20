// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

#![feature(associated_type_defaults)]

extern crate futures;

pub mod actor;

/// The actor prelude.
///
/// Useful traits and structs.
///
/// ```
/// use actor::prelude::*;
/// ```
pub mod prelude {
    pub use actor::{Actor, NewActor};
}
