//! Module containing the `Inbox` for an actor.

use std::collections::vec_deque;
use std::collections::VecDeque;
use std::iter::{Enumerate, FusedIterator};

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::rt::Waker;

pub(crate) mod oneshot;

#[cfg(test)]
mod tests;

/// Inbox that holds all messages for an actor.
#[derive(Debug)]
pub(crate) struct Inbox<M> {
    receiver: Receiver<M>,
}

impl<M> Inbox<M> {
    /// Create a new inbox. Needs a `waker` to wake the actor when sending
    /// messages to it.
    pub(crate) fn new(waker: Waker) -> (Inbox<M>, InboxRef<M>) {
        let (sender, receiver) = channel::unbounded();
        let inbox = Inbox { receiver };
        let inbox_ref = InboxRef { sender, waker };
        (inbox, inbox_ref)
    }

    /// Creates an `Inbox` for use in the `actor::Context`.
    pub(crate) fn ctx_inbox(&self) -> Inbox<M> {
        Inbox {
            receiver: self.receiver.clone(),
        }
    }

    /// Receive the next message, if any.
    pub(crate) fn receive_next(&mut self) -> Option<M> {
        self.receiver.try_recv().ok()
    }

    /*
    /// Receive a delivered message, if any.
    pub(crate) fn receive<S>(&mut self, _selector: &mut S) -> Option<M>
    where
        S: MessageSelector<M>,
    {
        self.with_messages(|messages| {
            selector
                .select(Messages::new(&messages))
                .and_then(|selection| messages.remove(selection.0))
        })
    }

    /// Peek the next delivered message, if any.
    pub(crate) fn peek_next(&mut self) -> Option<M>
    where
        M: Clone,
    {
        self.with_messages(|messages| messages.front().cloned())
    }

    /// Peek a delivered message, if any.
    pub(crate) fn peek<S>(&mut self, _selector: &mut S) -> Option<M>
    where
        S: MessageSelector<M>,
        M: Clone,
    {
        self.with_messages(|messages| {
            selector
                .select(Messages::new(&messages))
                .and_then(|selection| messages.get(selection.0).cloned())
        })
    }
    */
}

/// Reference to an actor's inbox, used to send messages to it.
#[derive(Debug)]
pub(crate) struct InboxRef<M> {
    sender: Sender<M>,
    waker: Waker,
}

impl<M> InboxRef<M> {
    /// Attempt to send a message to the mailbox.
    ///
    /// This will also mark the actor as ready to run.
    pub(crate) fn try_send(&self, msg: M) -> Result<(), M> {
        self.sender
            .try_send(msg)
            .map(|()| self.waker.wake())
            .map_err(|err| err.into_inner())
    }
}

impl<M> Clone for InboxRef<M> {
    fn clone(&self) -> InboxRef<M> {
        InboxRef {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }
}

// Below is re-exported in the `actor::message_select` module.

/// Trait that defines how to select what message to receive.
///
/// Implementations get passed an iterator that iterates over
/// [`MessageSelection`]s and references to messages. `MessageSelection` is the
/// type used to indicate what message to actually receive.
///
/// # Examples
///
/// The simplest possible implementation simply returns the first message, the
/// example below is the implementation behind [`First`].
///
/// ```
/// use heph::actor::message_select::{MessageSelection, MessageSelector, Messages};
///
/// struct First;
///
/// impl<M> MessageSelector<M> for First {
///     fn select<'m>(&mut self, mut messages: Messages<'m, M>) -> Option<MessageSelection> {
///         messages.next().map(|(selection, _)| selection)
///     }
/// }
///
/// # // Use `First` to silence dead code warning.
/// # drop(First);
/// ```
///
/// The following example selects the message with the highest priority (also
/// see [`Priority`]).
///
/// ```
/// use heph::actor::message_select::{MessageSelection, MessageSelector, Messages};
///
/// struct Message {
/// #   #[allow(dead_code)]
///     msg: String,
///     priority: usize,
/// }
///
/// struct MyMessageSelector;
///
/// impl MessageSelector<Message> for MyMessageSelector {
///     fn select<'m>(&mut self, messages: Messages<'m, Message>) -> Option<MessageSelection> {
///         messages.max_by_key(|(_, msg)| msg.priority).map(|(selection, _)| selection)
///     }
/// }
///
/// # // Use `MyMessageSelector` to silence dead code warning.
/// # drop(MyMessageSelector);
/// ```
pub trait MessageSelector<M> {
    /// Select what message to receive.
    fn select<'m>(&mut self, messages: Messages<'m, M>) -> Option<MessageSelection>;
}

/// The type used to indicate what message to select.
#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct MessageSelection(pub(crate) usize);

/// All message currently queued for an actor.
///
/// This is used by [`MessageSelector`] to select a message to receive next.
/// This implements [`Iterator`], iterating over [`MessageSelection`] and a
/// reference to the message `M`.
#[derive(Debug)]
pub struct Messages<'m, M> {
    inner: Enumerate<vec_deque::Iter<'m, M>>,
}

impl<'m, M> Messages<'m, M> {
    /// Create a new iterator for `messages`.
    pub(crate) fn new(messages: &'m VecDeque<M>) -> Messages<'m, M> {
        Messages {
            inner: messages.iter().enumerate(),
        }
    }
}

impl<'m, M> Iterator for Messages<'m, M> {
    type Item = (MessageSelection, &'m M);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(idx, msg)| (MessageSelection(idx), msg))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'m, M> DoubleEndedIterator for Messages<'m, M> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|(idx, msg)| (MessageSelection(idx), msg))
    }
}

impl<'m, M> ExactSizeIterator for Messages<'m, M> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<'m, M> FusedIterator for Messages<'m, M> {}

impl<F, M> MessageSelector<M> for F
where
    F: FnMut(&M) -> bool,
{
    fn select<'m>(&mut self, mut messages: Messages<'m, M>) -> Option<MessageSelection> {
        messages
            .find(|(_, msg)| (self)(msg))
            .map(|(selection, _)| selection)
    }
}

/// A [`MessageSelector`] implementation that selects the first message.
///
/// This will return the same message as [`actor::Context::receive_next`].
// and [`actor::Context::peek_next`].
///
/// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
// [`actor::Context::peek_next`]: crate::actor::Context::peek_next
/*
///
/// # Examples
///
/// ```
/// #![feature(never_type)]
///
/// use heph::actor::message_select::First;
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
///
/// fn main() -> Result<(), rt::Error> {
///     Runtime::new().with_setup(setup).start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let actor_ref = runtime_ref.spawn(NoSupervisor, actor as fn(_) -> _, (),
///         ActorOptions::default());
///
///     // We'll send our actor two messages.
///     actor_ref.send("Message 1".to_owned()).unwrap();
///     actor_ref.send("Message 2".to_owned()).unwrap();
///     Ok(())
/// }
///
/// async fn actor(mut ctx: actor::Context<String>) -> Result<(), !> {
///     // Using `First` is the same as the `{peek, retrieve}_next` functions.
///     let msg1 = ctx.peek_next().await;
///     let msg1_again = ctx.receive(First).await;
///     assert_eq!(msg1, msg1_again);
/// #   assert_eq!(msg1, "Message 1");
///     println!("Got message: {}", msg1);
/// #   // Also check the second message.
/// #   let msg = ctx.receive(First).await;
/// #   assert_eq!(msg, "Message 2");
///     Ok(())
/// }
/// ```
*/
#[derive(Debug)]
pub struct First;

impl<M> MessageSelector<M> for First {
    fn select<'m>(&mut self, mut messages: Messages<'m, M>) -> Option<MessageSelection> {
        messages.next().map(|(selection, _)| selection)
    }
}

/// Select a message with the highest priority.
/*
///
/// # Examples
///
/// ```
/// #![feature(never_type)]
///
/// use heph::actor::message_select::Priority;
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
///
/// fn main() -> Result<(), rt::Error> {
///     Runtime::new().with_setup(setup).start()
/// }
///
/// fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
///     let actor_ref = runtime_ref.spawn(NoSupervisor, actor as fn(_) -> _, (),
///         ActorOptions::default());
///
///     // We'll send our actor two messages, one normal one and a priority one.
///     actor_ref.send(Message {
///         priority: 1,
///         msg: "Normal message".to_owned(),
///     }).unwrap();
///     actor_ref.send(Message {
///         priority: 100,
///         msg: "Priority message".to_owned(),
///     }).unwrap();
///     Ok(())
/// }
///
/// struct Message {
///     priority: usize,
///     msg: String,
/// }
///
/// async fn actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
///     // As both messages are ready this will receive the priority message.
///     let msg = ctx.receive(Priority(|msg: &Message| msg.priority)).await;
/// #   assert_eq!(msg.priority, 100);
/// #   assert_eq!(msg.msg, "Priority message");
///     println!("Got message: {}", msg.msg);
/// #   // Also check the second message.
/// #   let msg = ctx.receive(Priority(|msg: &Message| msg.priority)).await;
/// #   assert_eq!(msg.priority, 1);
/// #   assert_eq!(msg.msg, "Normal message");
///     Ok(())
/// }
/// ```
*/
#[derive(Debug)]
pub struct Priority<F>(pub F);

impl<F, P, M> MessageSelector<M> for Priority<F>
where
    F: FnMut(&M) -> P,
    P: Ord,
{
    fn select<'m>(&mut self, messages: Messages<'m, M>) -> Option<MessageSelection> {
        messages
            .max_by_key(|(_, msg)| (self.0)(msg))
            .map(|(selection, _)| selection)
    }
}
