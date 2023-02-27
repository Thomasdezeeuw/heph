//! Various [`Route`]rs.
//!
//! The following routes are provided:
//!  * [`Relay`] relays all messages to a single actor.
//!  * [`RelayGroup`] relays all messages to a group of actors.
//!  * [`Drop`] drops all messages.

use std::fmt;
use std::future::Future;
use std::future::{ready, Ready};
use std::net::SocketAddr;

use heph::actor_ref::{ActorGroup, ActorRef, Delivery, SendError, SendValue};

use crate::net_relay::Route;

impl<F, M, Fut, E> Route<M> for F
where
    F: FnMut(M, SocketAddr) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: fmt::Display,
{
    type Route<'a> = Fut
        where Self: 'a;
    type Error = E;

    fn route<'a>(&'a mut self, msg: M, source: SocketAddr) -> Self::Route<'a> {
        (self)(msg, source)
    }
}

/// [`Route`] implementation that routes all messages to a single
/// actor.
#[derive(Debug)]
pub struct Relay<M> {
    actor_ref: ActorRef<M>,
}

impl<M> Relay<M> {
    /// Relay all remote messages to the `actor_ref`.
    pub const fn to(actor_ref: ActorRef<M>) -> Relay<M> {
        Relay { actor_ref }
    }
}

impl<M> Clone for Relay<M> {
    fn clone(&self) -> Relay<M> {
        Relay {
            actor_ref: self.actor_ref.clone(),
        }
    }
}

impl<M> Route<M> for Relay<M>
where
    M: 'static + Unpin,
{
    type Error = SendError;
    type Route<'a> = SendValue<'a, M>
        where Self: 'a;

    fn route<'a>(&'a mut self, msg: M, _: SocketAddr) -> Self::Route<'a> {
        self.actor_ref.send(msg)
    }
}

/// [`Route`] implementation that routes all messages to a group of
/// actors.
#[derive(Debug)]
pub struct RelayGroup<M> {
    actor_group: ActorGroup<M>,
    delivery: Delivery,
}

impl<M> RelayGroup<M> {
    /// Relay all remote messages to the `actor_group`.
    pub const fn to(actor_group: ActorGroup<M>, delivery: Delivery) -> RelayGroup<M> {
        RelayGroup {
            actor_group,
            delivery,
        }
    }
}

impl<M> Clone for RelayGroup<M> {
    fn clone(&self) -> RelayGroup<M> {
        RelayGroup {
            actor_group: self.actor_group.clone(),
            delivery: self.delivery,
        }
    }
}

impl<M> Route<M> for RelayGroup<M>
where
    M: Clone + Unpin + 'static,
{
    type Error = !;
    type Route<'a> = Ready<Result<(), Self::Error>>
        where Self: 'a;

    fn route<'a>(&'a mut self, msg: M, _: SocketAddr) -> Self::Route<'a> {
        _ = self.actor_group.try_send(msg, self.delivery);
        ready(Ok(()))
    }
}

/// Router that drops all messages.
#[derive(Copy, Clone, Debug)]
pub struct Drop;

impl<M> Route<M> for Drop {
    type Error = !;
    type Route<'a> = Ready<Result<(), Self::Error>>
        where Self: 'a;

    fn route<'a>(&'a mut self, _: M, _: SocketAddr) -> Self::Route<'a> {
        ready(Ok(()))
    }
}
