//! Channel communication primitives.
//!
//! These channels come in three flavours:
//!
//! - *One shot*, a channel that allows a single value to be send across.
//! - *Bounded*, a channel that buffers a maximum number of values in the
//!   channel.
//! - *Unbounded*, a channel with a buffer that grows infinitely.
//!
//! All three channels flavours are **not** thread safe, they don't implement
//! [`Send`] or [`Sync`]. If thread safe channels are required the standard
//! library provides a channel in the [`std::sync::mpsc`] module, or look at the
//! [`crossbeam_channels`] crate.
//!
//! All three flavours follow roughly the same API. Each submodule has a
//! `channel` function that can be used to create the `Sender` and `Receiver`
//! halves of the channel. Alternatively the [`oneshot`], [`bounded`] and
//! [`unbounded`] functions can be used to create the two halves. Each `Sender`
//! has a `send` method, which sends a value across the channel. `Sender` also
//! has a `is_connected` method to check if the other half is still connected,
//! useful before doing an expensive computation and then sending the result
//! into an disconnected channel. Each `Receiver` either implements `Future` or
//! `Stream` to receive values from the channel. The bounded and unbounded
//! flavours also have a `receive_one` method that returns a `Future` that
//! receives a single value from the channel.
//!
//! [`crossbeam_channels`]: https://crates.io/crates/crossbeam-channel
//! [`std::sync::mpsc`]: https://doc.rust-lang.org/nightly/std/sync/mpsc/index.html
//! [`oneshot`]: fn.oneshot.html
//! [`bounded`]: fn.bounded.html
//! [`unbounded`]: fn.unbounded.html
//!
//! # When to use which flavour
//!
//! When you expect a maximum of one value to be send across the channel the
//! answer is simple, use the [oneshot] channel. For example when using the
//! [request-response pattern].
//!
//! When more then one value is expected to be send the choice gets a little
//! harder. If you don't want the producer (sender) to be blocked then the
//! [unbounded] channel is the way to go. However if the consumer (receiver)
//! can't keep up with the values it might better to limit the pace at which the
//! producer produces value, in that case a [bounded] channel is the way to go.
//!
//! [oneshot]: oneshot/index.html
//! [request-response pattern]: oneshot/index.html#request-response-pattern
//! [unbounded]: unbounded/index.html
//! [bounded]: bounded/index.html

use std::pin::Unpin;

pub mod bounded;
pub mod oneshot;
pub mod unbounded;

/// Receiving half of the channel was dropped.
///
/// The send value can be retrieved.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct NoReceiver<T>(pub T);

/// Sending halves of the channel were dropped, without more values being
/// available.
///
/// In case of a oneshot channel it means that no value was ever send. In case
/// of bounded and unbounded channels it means no more values will be send and
/// all send values were received.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct NoValue;

/// Create a new bounded channel.
///
/// See the [`bounded`] module for more information.
///
/// [`bounded`]: bounded/index.html
pub fn bounded<T: Unpin>(capacity: usize) -> (bounded::Sender<T>, bounded::Receiver<T>) {
    bounded::channel(capacity)
}

/// Create a new oneshot channel.
///
/// See the [`oneshot`] module for more information.
///
/// [`oneshot`]: oneshot/index.html
pub fn oneshot<T: Unpin>() -> (oneshot::Sender<T>, oneshot::Receiver<T>) {
    oneshot::channel()
}

/// Create a new unbounded channel.
///
/// See the [`unbounded`] module for more information.
///
/// [`unbounded`]: unbounded/index.html
pub fn unbounded<T: Unpin>() -> (unbounded::Sender<T>, unbounded::Receiver<T>) {
    unbounded::channel()
}
