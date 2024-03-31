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

use heph::actor_ref::{ActorGroup, ActorRef, SendError, SendValue};

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

/// The kind of delivery to use.
#[derive(Copy, Clone, Debug)]
pub enum Delivery {
    /// Delivery a copy of the message to all actors in the group.
    ToAll,
    /// Delivery the message to one of the actors.
    ToOne,
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
        _ = match self.delivery {
            Delivery::ToAll => self.actor_group.try_send_to_all(msg),
            Delivery::ToOne => self.actor_group.try_send_to_one(msg),
        };
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
