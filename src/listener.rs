// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::io;

use futures::Poll;
use mio::event::Evented;

/// The Listener trait.
///
/// Accepting another item on listener is the start of another iteration in the
/// event loop. This can be a new TCP connection coming in, or new UDP packet
/// arriving, but it can also be bytes coming in through some I/O pipe (e.g.
/// standard in).
// TODO: Hide the Evented requirement?
pub trait Listener: Evented {
    /// The type of item this listener will return, e.g. a TCP connection for a
    /// TCP listener.
    type Item;

    /// Accept a new item, e.g. a new TCP connection. If no new items are ready
    /// `Async::NotReady` should be returned.
    fn accept(&mut self) -> Poll<Self::Item, io::Error>;

    // TODO: add start and stop methods?
}

/// The trait to create a new [`Listener`] for each thread.
///
/// [`Listener`]: trait.Listener.html
pub trait NewListener {
    /// The type of listener, see the trait documentation for more.
    type Listener: Listener;

    /// Create a new listener for a thread.
    fn new(&self) -> Result<Self::Listener, io::Error>;
}
