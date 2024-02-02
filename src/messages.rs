//! Standardised messages.
//!
//! To use these message the receiving actor should implement [`From`]`<Message>`,
//! this way the sending actor can simply send the message, without having to
//! wrap it in a message type first. See the examples below. The `From`
//! implementations can also automated by the [`from_message!`] macro.
//!
//! Most message types have an optional id, defaulting to `()`. This allows a
//! single actor to receive messages from multiple sources with the ability to
//! differentiate the source of the message.
//!
//! Three more default messages should be considered, the two variants from
//! [`Result`]: [`Ok`] and [`Err`]. Lastly there is the `Signal` type from the
//! `heph-rt` crate to handle process signals.
//!
//! # Examples
//!
//! Implementing `From<Message>` manually to allow for easy sending of messages.
//! Also see the [`from_message!`] macro to automate this.
//!
//! ```
//! use heph::messages::Ack;
//!
//! #[derive(Debug, Eq, PartialEq)]
//! struct OK;
//!
//! #[derive(Debug, Eq, PartialEq)]
//! struct Error;
//!
//! /// The message type for the coordinating actor.
//! #[derive(Debug, Eq, PartialEq)]
//! enum Message {
//!     /// Acknowledgement of receiving an message.
//!     Ack(usize),
//!     /// An ok result.
//!     Ok(OK),
//!     /// An erroneous result.
//!     Error(Error),
//! }
//!
//! // This allows us to receive an `Ack` message.
//! impl From<Ack<usize>> for Message {
//!     fn from(ack: Ack<usize>) -> Message {
//!         Message::Ack(ack.0)
//!     }
//! }
//!
//! // Abilities to receive an result from a working actor.
//! impl From<Result<OK, Error>> for Message {
//!     fn from(res: Result<OK, Error>) -> Message {
//!         match res {
//!             Ok(ok) => Message::Ok(ok),
//!             Err(err) => Message::Error(err),
//!         }
//!     }
//! }
//! #
//! # drop(Message::Ack(0));
//! ```

/// A start signal
///
/// Useful for example when you want to delay the start of an actor. This
/// message has an optional id.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Start<Id = ()>(pub Id);

/// An acknowledgement.
///
/// Useful for example when you want to know if a message was received. This
/// message has an optional id.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Ack<Id = ()>(pub Id);

/// Signal to an actor that we're done.
///
/// This message has an optional id.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Done<Id = ()>(pub Id);

/// Signal to an actor to cancel an operation.
///
/// This message has an optional id.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Cancel<Id = ()>(pub Id);

/// Ask an actor to terminate.
///
/// # Notes
///
/// This message is not special in anyway, this means the receiving actor can
/// simply ignore this message and continue running.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Terminate;

/// Macro to implement [`From`] for an enum message type.
///
/// # Examples
///
/// ```
/// # #![allow(dead_code)]
/// use heph::actor_ref::RpcMessage;
/// use heph::from_message;
///
/// #[derive(Debug)]
/// enum Message {
///     Msg(String),
///     Rpc(RpcMessage<String, usize>),
///     Rpc2(RpcMessage<(String, usize), (usize, usize)>),
/// }
///
/// // This implements `From<String>` for `Message`.
/// from_message!(Message::Msg(String));
///
/// // RPC is also supported:
/// from_message!(Message::Rpc(String) -> usize);
/// from_message!(Message::Rpc2(String, usize) -> (usize, usize));
/// ```
#[macro_export]
macro_rules! from_message {
    // Single field message.
    ($name: ident :: $variant: ident ( $ty: ty )) => {
        impl From<$ty> for $name {
            fn from(msg: $ty) -> $name {
                $name::$variant(msg)
            }
        }
    };
    // Single field RPC.
    ($name: ident :: $variant: ident ( $ty: ty ) -> $return_ty: ty) => {
        impl From<$crate::actor_ref::rpc::RpcMessage<$ty, $return_ty>> for $name {
            fn from(msg: $crate::actor_ref::rpc::RpcMessage<$ty, $return_ty>) -> $name {
                $name::$variant(msg)
            }
        }
    };
    // Multiple values RPC, for which we use the tuple format.
    ($name: ident :: $variant: ident ( $( $ty: ty ),+ ) -> $return_ty: ty) => {
        impl From<$crate::actor_ref::rpc::RpcMessage<( $( $ty ),+ ), $return_ty>> for $name {
            fn from(msg: $crate::actor_ref::rpc::RpcMessage<( $( $ty ),+ ), $return_ty>) -> $name {
                $name::$variant(msg)
            }
        }
    };
}

pub use from_message;
