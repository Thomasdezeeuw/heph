//! The module with the `NewInitiator` and `Initiator` trait definitions.

use std::io;

use mio_st::poll::Poller;

use crate::scheduler::ProcessId;
use crate::system::ActorSystemRef;

/// The trait that defines how a new [`Initiator`] is created.
///
/// `NewInitiator` is to [`Initiator`] what [`NewActor`] is to [`Actor`], It
/// defines how a new `Initiator` is created. For each worker thread the
/// `NewInitiator` will be cloned, hence the `Clone` requirement, and send to
/// the worker thread, hence the `Send` requirement.
///
/// The [`new`] method will then be called on each worker thread, consuming the
/// `NewInitiator` implementation.
///
/// [`Initiator`]: trait.Initiator.html
/// [`NewActor`]: ../actor/trait.NewActor.html
/// [`Actor`]: ../actor/trait.Actor.html
/// [`new`]: #tymethod.new
pub trait NewInitiator: Send + Clone {
    /// The type of the initiator.
    ///
    /// See [`Initiator`] for more.
    ///
    /// [`Initiator`]: trait.Initiator.html
    type Initiator: Initiator;

    /// Create a new [`Initiator`].
    ///
    /// Will be called on the worker thread the `Initiator` needs to run on. The
    /// `Initiator` must be fully initialised and ready to be polled once
    /// returned.
    ///
    /// [`Initiator`]: trait.Initiator.html
    ///
    /// # Notes
    ///
    /// Most types do **not** support calling `new` directly and should be seen
    /// as an internally callable API only.
    fn new(self) -> io::Result<Self::Initiator>;

    /// This the method that actually gets called to create a new `Initiator`.
    ///
    /// `new` is only here to make it possible to implement this trait outside
    /// the crate, as both `Poller` and `ProcessId` are not part of the public
    /// API.
    ///
    /// For internal types this method use be implemented and `unreachable!`
    /// should be used for `new`.
    #[doc(hidden)]
    fn new_internal(self, _poller: &mut Poller, _pid: ProcessId) -> io::Result<Self::Initiator> {
        self.new()
    }
}

/// The `Initiator` is responsible for initiating events in the actor system.
///
/// Implementations of this trait can be found [below]. This includes a TCP
/// listener that will create a new actor for each incoming connection.
///
/// [below]: #implementors
pub trait Initiator {
    /// Poll the `Initiator` for new events.
    fn poll(&mut self, system_ref: &mut ActorSystemRef) -> io::Result<()>;
}

/// This is used by `ActorSystemBuilder` when no initiators are used.
///
/// Consider this an implementation detail, not a part of the public API.
#[doc(hidden)]
impl NewInitiator for ! {
    type Initiator = !;

    fn new(self) -> io::Result<Self::Initiator> {
        self
    }
}

/// This is used by `ActorSystemBuilder` when no initiators are used.
///
/// Consider this an implementation detail, not a part of the public API.
#[doc(hidden)]
impl Initiator for ! {
    fn poll(&mut self, _system_ref: &mut ActorSystemRef) -> io::Result<()> {
        *self
    }
}
