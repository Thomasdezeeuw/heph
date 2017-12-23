// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

//use super::super::num_cpus;
use super::ActorSystem;

pub struct ActorSystemBuilder {
    //n_threads: usize,
}

impl ActorSystemBuilder {
    pub fn build<'a>(self) -> ActorSystem<'a> {
        // TODO: log system spec in debug.
        ActorSystem {
            current_id: 0,
            runners: Default::default(),
        }
    }
}

impl Default for ActorSystemBuilder {
    fn default() -> ActorSystemBuilder {
        ActorSystemBuilder {
            //n_threads: num_cpus::get(),
        }
    }
}
