use std::future::Future;
use std::mem::PinMut;
use std::task::{Context, Poll};

use crate::actor::{Actor, ActorContext};
use crate::system::ActorRef;

/// A group of actors.
///
/// `ActorGroup` represents a group of actors to which one can send messages.
/// Message can either be broadcasts, send to all actors within the group, or
/// send to a single one, in a load balancing fashion.
///
/// The group can be created using [`new`] or using the [`From`] implementation.
///
/// # Notes
///
/// The [`Actor::Message`] must implement `Clone` to support broadcasting the
/// value.
///
/// [`new`]: #method.new
/// [`From`]: #impl-From%3CVec%3CActorRef%3CA%3E%3E%3E
/// [`Actor::Message`]: trait.Actor.html#associatedtype.Message
#[derive(Debug)]
pub struct ActorGroup<M> {
    /// The context we're working in.
    actor_ctx: ActorContext<GroupMessage<M>>,
    /// The actors within the group.
    actors: Vec<ActorRef<M>>,
    /// Index of the next actor to send the `GroupMessage::One` to.
    next_index: usize,
}

/// Message type for [`ActorGroup`].
///
/// [`ActorGroup`]: ./struct.ActorGroup.html
#[derive(Debug)]
#[non_exhaustive]
pub enum GroupMessage<M> {
    /// Broadcast a message to all actors within the group.
    Broadcast(M),
    /// Send a message to a single actor within the group, in a round-robin
    /// fashion.
    One(M),
    /// Add an actor to the group.
    AddActor(ActorRef<M>),
    // TODO: removing actors?
}

impl<M> Actor for ActorGroup<M>
    where M: Clone,
{
    type Error = !;

    fn try_poll(self: PinMut<Self>, ctx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let this = unsafe { PinMut::get_mut_unchecked(self) };
        loop {
            let mut future = this.actor_ctx.receive();
            let future = unsafe { PinMut::new_unchecked(&mut future) };
            let msg = match future.poll(ctx) {
                Poll::Ready(msg) => msg,
                Poll::Pending => return Poll::Pending,
            };
            match msg {
                GroupMessage::Broadcast(msg) => {
                    for actor in &mut this.actors {
                        // TODO: handle error.
                        let _ = actor.send(msg.clone());
                    }
                },
                GroupMessage::One(msg) => {
                    loop {
                        match this.actors.get_mut(this.next_index) {
                            Some(actor_ref) => {
                                // TODO: handle error.
                                let _ = actor_ref.send(msg);
                                this.next_index += 1;
                                break;
                            },
                            None => {
                                this.next_index = 0;
                            }
                        }
                    }
                },
                GroupMessage::AddActor(actor_ref) => {
                    this.actors.push(actor_ref);
                },
            }
        }
    }
}
