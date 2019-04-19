//! Module containing the `MailBox` for an actor.

use std::collections::VecDeque;
use std::collections::vec_deque;
use std::iter::{Enumerate, FusedIterator};

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::scheduler::ProcessId;
use crate::system::ActorSystemRef;

/// Mailbox that holds all messages for an actor.
#[derive(Debug)]
pub struct MailBox<M> {
    /// Process id of the actor, used to notify the actor when receiving
    /// message.
    pid: ProcessId,
    /// Reference to the actor system, used to notify the actor.
    system_ref: ActorSystemRef,
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// This is an alternative source of messages, send across thread bounds,
    /// used by machine local actor references to send messages. This defaults
    /// to `None` and is only set to `Some` the first time `upgrade_ref` is
    /// called.
    messages2: Option<(Sender<M>, Receiver<M>)>,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    pub fn new(pid: ProcessId, system_ref: ActorSystemRef) -> MailBox<M> {
        MailBox {
            pid,
            system_ref,
            messages: VecDeque::new(),
            messages2: None,
        }
    }

    /// Deliver a new message to the mailbox.
    ///
    /// This will also schedule the actor to run.
    pub fn deliver(&mut self, msg: M) {
        self.system_ref.notify(self.pid);
        self.messages.push_back(msg);
    }

    /// Receive the next message, if any.
    pub fn receive_next(&mut self) -> Option<M> {
        self.receive_remote_messages();
        self.messages.pop_front()
    }

    /// Receive a delivered message, if any.
    pub fn receive<S>(&mut self, selector: &mut S) -> Option<M>
        where S: MessageSelector<M>,
    {
        self.receive_remote_messages();
        let messages = self.messages.iter().enumerate();
        selector.select(Messages{ inner: messages })
            .and_then(|selection| self.messages.remove(selection.0))
    }

    /// Receive all remote messages, if any, and add them to the message queue.
    ///
    /// This is done in an attempt to make the receiving of the two sources of
    /// message a bit more fair.
    fn receive_remote_messages(&mut self) {
        if let Some((_, recv)) = self.messages2.as_ref() {
            while let Ok(msg) = recv.try_recv() {
                self.messages.push_back(msg);
            }
        }
    }

    /// Used by local actor reference to upgrade to a machine local actor
    /// reference.
    pub fn upgrade_ref(&mut self) -> (ProcessId, Sender<M>) {
        (self.pid, self.messages2.get_or_insert_with(channel::unbounded).0.clone())
    }
}

// Below is re-exported in the `actor::context` module.

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
/// use heph::actor::message_select::{Messages, MessageSelector, MessageSelection};
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
/// The following example selects the message with the highest priority.
///
/// ```
/// use heph::actor::message_select::{Messages, MessageSelector, MessageSelection};
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
///         messages
///             .max_by_key(|(_, msg)| msg.priority)
///             .map(|(selection, _)| selection)
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
pub struct MessageSelection(usize);

/// All message currently queued for an actor.
///
/// This is used by [`MessageSelector`] to select a message to receive next.
/// This implements [`Iterator`], iterating over [`MessageSelection`] and a
/// reference to the message.
#[derive(Debug)]
pub struct Messages<'m, M> {
    inner: Enumerate<vec_deque::Iter<'m, M>>,
}

impl<'m, M> Iterator for Messages<'m, M> {
    type Item = (MessageSelection, &'m M);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
            .map(|(idx, msg)| (MessageSelection(idx), msg))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'m, M> DoubleEndedIterator for Messages<'m, M> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back()
            .map(|(idx, msg)| (MessageSelection(idx), msg))
    }
}

impl<'m, M> ExactSizeIterator for Messages<'m, M> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<'m, M> FusedIterator for Messages<'m, M> { }

impl<F, M> MessageSelector<M> for F
    where F: FnMut(&M) -> bool,
{
    fn select<'m>(&mut self, messages: Messages<'m, M>) -> Option<MessageSelection> {
        for (idx, msg) in messages {
            if (self)(msg) {
                return Some(idx);
            }
        }
        None
    }
}

/// A [`MessageSelector`] implementation that selects the first message.
///
/// Used by [`actor::Context::receive_next`].
///
/// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
#[derive(Debug)]
pub struct First;

impl<M> MessageSelector<M> for First {
    fn select<'m>(&mut self, mut messages: Messages<'m, M>) -> Option<MessageSelection> {
        messages.next().map(|(selection, _)| selection)
    }
}

/// Select a message with the highest priority.
///
/// # Examples
///
/// ```
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use heph::actor::message_select::Priority;
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef};
///
/// ActorSystem::new()
///     .with_setup(setup)
///     .run()
///     .unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
///     let mut actor_ref = system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (),
///         ActorOptions::default());
///
///     // We'll send our actor two messages, one normal one and a priority one.
///     actor_ref <<= Message { priority: 1, msg: "Normal message".to_owned() };
///     actor_ref <<= Message { priority: 100, msg: "Priority message".to_owned() };
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
///     let msg = await!(ctx.receive(Priority(|msg: &Message| msg.priority)));
///     println!("Got message: {}", msg.msg);
///     Ok(())
/// }
///
/// # // Use the `actor` function to silence dead code warning.
/// # drop(actor);
/// ```
#[derive(Debug)]
pub struct Priority<F>(pub F);

impl<F, P, M> MessageSelector<M> for Priority<F>
    where F: FnMut(&M) -> P,
          P: Ord,
{
    fn select<'m>(&mut self, messages: Messages<'m, M>) -> Option<MessageSelection> {
        messages
            .max_by_key(|(_, msg)| (self.0)(msg))
            .map(|(selection, _)| selection)
    }
}
