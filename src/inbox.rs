//! Module containing the `MailBox` for an actor.

use std::cell::RefCell;
use std::collections::vec_deque;
use std::collections::VecDeque;
use std::iter::{Enumerate, FusedIterator};
use std::rc::{Rc, Weak};

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::system::ActorSystemRef;
use crate::system::ProcessId;

/// Inbox that holds all messages for an actor.
#[derive(Debug)]
pub struct Inbox<M> {
    shared: Rc<RefCell<SharedInbox<M>>>,
}

#[derive(Debug)]
struct SharedInbox<M> {
    /// Process id of the actor, used to notify the actor when receiving
    /// message.
    pid: ProcessId,
    /// Reference to the actor system, used to notify the actor.
    system_ref: ActorSystemRef,
    /// The messages in the mailbox.
    ///
    /// NOTE: don't use this directly instead used `with_messages`.
    messages: VecDeque<M>,
    /// This is an alternative source of messages, send across thread bounds,
    /// used by machine local actor references to send messages. This defaults
    /// to `None` and is only set to `Some` the first time `upgrade_ref` is
    /// called.
    ///
    /// NOTE: don't use this directly instead used `with_messages`.
    messages2: Option<(Sender<M>, Receiver<M>)>,
}

impl<M> Inbox<M> {
    /// Create a new inbox.
    pub fn new(pid: ProcessId, system_ref: ActorSystemRef) -> Inbox<M> {
        Inbox {
            shared: Rc::new(RefCell::new(SharedInbox {
                pid,
                system_ref,
                messages: VecDeque::new(),
                messages2: None,
            })),
        }
    }

    /// Create a reference to this inbox.
    pub fn create_ref(&self) -> InboxRef<M> {
        InboxRef {
            shared: Rc::downgrade(&self.shared),
        }
    }

    /// Receive the next message, if any.
    pub fn receive_next(&mut self) -> Option<M> {
        self.with_messages(|messages| messages.pop_front())
    }

    /// Receive a delivered message, if any.
    pub fn receive<S>(&mut self, selector: &mut S) -> Option<M>
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
    pub fn peek_next(&mut self) -> Option<M>
    where
        M: Clone,
    {
        self.with_messages(|messages| messages.front().cloned())
    }

    /// Peek a delivered message, if any.
    pub fn peek<S>(&mut self, selector: &mut S) -> Option<M>
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

    /// Receive all remote messages, if any, and add them to the message queue
    /// and then calls `f` with all currently available messages.
    ///
    /// This is done in an attempt to make the receiving of the two sources of
    /// message a bit more fair.
    fn with_messages<F>(&mut self, f: F) -> Option<M>
    where
        F: FnOnce(&mut VecDeque<M>) -> Option<M>,
    {
        let mut shared = self.shared.borrow_mut();
        let shared = &mut *shared;
        if let Some((_, recv)) = shared.messages2.as_ref() {
            while let Ok(msg) = recv.try_recv() {
                shared.messages.push_back(msg);
            }
        }
        f(&mut shared.messages)
    }
}

impl<M> Clone for Inbox<M> {
    fn clone(&self) -> Inbox<M> {
        Inbox {
            shared: Rc::clone(&self.shared),
        }
    }
}

/// Reference to an inbox that may fail to get access to the inbox if the actor
/// is stopped.
#[derive(Debug)]
pub struct InboxRef<M> {
    shared: Weak<RefCell<SharedInbox<M>>>,
}

impl<M> InboxRef<M> {
    /// Attempt to deliver a message to the mailbox.
    ///
    /// This will also schedule the actor to run.
    pub fn try_deliver(&mut self, msg: M) -> Result<(), M> {
        match self.shared.upgrade() {
            Some(shared) => {
                let mut shared = shared.borrow_mut();
                let pid = shared.pid;
                shared.messages.push_back(msg);
                shared.system_ref.notify(pid);
                Ok(())
            }
            None => Err(msg),
        }
    }

    /// Used by local actor reference to upgrade to a machine local actor
    /// reference.
    pub fn try_upgrade_ref(&mut self) -> Result<(ProcessId, Sender<M>), ()> {
        match self.shared.upgrade() {
            Some(shared) => {
                let mut shared = shared.borrow_mut();
                let sender = shared
                    .messages2
                    .get_or_insert_with(channel::unbounded)
                    .0
                    .clone();
                Ok((shared.pid, sender))
            }
            None => Err(()),
        }
    }

    /// Returns true if `self` and `other` point to the same inbox.
    pub fn same_inbox(&self, other: &InboxRef<M>) -> bool {
        Weak::ptr_eq(&self.shared, &other.shared)
    }

    #[cfg(test)]
    pub fn upgrade(&mut self) -> Option<Inbox<M>> {
        self.shared.upgrade().map(|shared| Inbox { shared })
    }
}

impl<M> Clone for InboxRef<M> {
    fn clone(&self) -> InboxRef<M> {
        InboxRef {
            shared: Weak::clone(&self.shared),
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
/// The following example selects the message with the highest priority.
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
/// reference to the message.
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
/// Used by [`actor::Context::receive_next`].
///
/// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
///
/// # Examples
///
/// ```
/// #![feature(async_await, never_type)]
///
/// use heph::actor::message_select::First;
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef};
///
/// ActorSystem::new().with_setup(setup).run().unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
///     let mut actor_ref = system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (), ActorOptions::default());
///
///     // We'll send our actor two messages.
///     actor_ref <<= "Message 1".to_owned();
///     actor_ref <<= "Message 2".to_owned();
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
/// #![feature(async_await, never_type)]
///
/// use heph::actor::message_select::Priority;
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef};
///
/// ActorSystem::new().with_setup(setup).run().unwrap();
///
/// fn setup(mut system_ref: ActorSystemRef) -> Result<(), !> {
///     let mut actor_ref = system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (), ActorOptions::default());
///
///     // We'll send our actor two messages, one normal one and a priority one.
///     actor_ref <<= Message {
///         priority: 1,
///         msg: "Normal message".to_owned(),
///     };
///     actor_ref <<= Message {
///         priority: 100,
///         msg: "Priority message".to_owned(),
///     };
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
