//! Module with the UDP implementation of the net relay.

use std::io;
use std::net::SocketAddr;
use std::pin::pin;

use heph::actor::{self, NoMessages};
use heph::messages::Terminate;
use heph_rt::net::UdpSocket;
use heph_rt::util::either;
use heph_rt::{self as rt, Signal};
use log::warn;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use crate::net_relay::uuid::UuidGenerator;
use crate::net_relay::{Message, Route, Serde};

const MAX_PACKET_SIZE: usize = 1 << 16; // ~65kb.
const INITIAL_SEND_BUF_SIZE: usize = 1 << 12; // 4kb.

/// Message type used for network relays using UDP.
///
/// # Notes
///
/// When using an [`ActorRef`] with this type you can use [`ActorRef::map_fn`]
/// to e.g. always add an address and change the message type to something more
/// convenient.
///
/// ```
/// # #![allow(dead_code)]
/// use heph::ActorRef;
/// use heph_remote::net_relay::UdpRelayMessage;
///
/// # return;
/// // Original actor reference from spawing the relay actor.
/// let actor_ref: ActorRef<UdpRelayMessage<String>> = // ...
/// # todo!();
///
/// // A remote target we want to send our messages to.
/// let target = "127.0.0.1:8080".parse().unwrap();
/// // Using the `map_fn` we can always set the target and use the `String` type
/// // as message.
/// let actor_ref: ActorRef<String> = actor_ref
///     .map_fn(move |message| UdpRelayMessage::Relay { message, target });
/// ```
///
/// [`ActorRef`]: heph::ActorRef
/// [`ActorRef::map_fn`]: heph::ActorRef::map_fn
#[derive(Debug)]
pub enum UdpRelayMessage<M> {
    /// Relay message `M` to `target`.
    Relay {
        /// Message to send.
        message: M,
        /// Target to send the message to.
        target: SocketAddr,
    },
    /// Stop the relay.
    Terminate,
}

impl<M> From<Terminate> for UdpRelayMessage<M> {
    fn from(_: Terminate) -> UdpRelayMessage<M> {
        UdpRelayMessage::Terminate
    }
}

impl<M> TryFrom<Signal> for UdpRelayMessage<M> {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(UdpRelayMessage::Terminate),
            _ => Err(()),
        }
    }
}

/// Actor that handles remote messages.
///
/// It receives `Out`going messages from it's inbox and sends them to a remote
/// actor using UDP. Any `In`coming message on the same socket will be routed
/// using the `R`outer.
pub(crate) async fn remote_relay<S, Out, In, R, RT>(
    mut ctx: actor::Context<UdpRelayMessage<Out>, RT>,
    local_address: SocketAddr,
    mut router: R,
) -> io::Result<()>
where
    S: Serde,
    Out: Serialize,
    In: DeserializeOwned,
    RT: rt::Access,
    R: Route<In>,
{
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;

    let mut uuid_gen = UuidGenerator::new();
    let mut send_buf = Vec::with_capacity(INITIAL_SEND_BUF_SIZE);

    let mut recv_data = pin!(socket.recv_from(Vec::with_capacity(MAX_PACKET_SIZE)));
    loop {
        match either(ctx.receive_next(), recv_data.as_mut()).await {
            // Received an outgoing message we want to relay to a remote
            // actor.
            Ok(Ok(UdpRelayMessage::Relay { message, target })) => {
                send_buf =
                    send_message::<S, Out>(&socket, send_buf, &mut uuid_gen, target, &message)
                        .await?;
                send_buf.clear();
            }
            Ok(Ok(UdpRelayMessage::Terminate) | Err(NoMessages)) => return Ok(()),
            // Received an incoming packet.
            Err(Ok((mut buf, source))) => {
                route_message::<S, R, In>(&mut router, &buf, source).await?;
                buf.clear();
                recv_data.set(socket.recv_from(buf));
            }
            // Error receiving a packet.
            Err(Err(err)) => return Err(err),
        }
    }
}

/// Send a `msg` to a remote actor at `target` address, using `socket`.
async fn send_message<S, M>(
    socket: &UdpSocket,
    mut buf: Vec<u8>,
    uuid_gen: &mut UuidGenerator,
    target: SocketAddr,
    msg: &M,
) -> io::Result<Vec<u8>>
where
    S: Serde,
    M: Serialize,
{
    // Serialise the message to our buffer first.
    let uuid = uuid_gen.next();
    let msg = Message { uuid, msg };
    if let Err(err) = S::to_buf(&mut buf, &msg) {
        warn!("error serialising message (for {target}): {err}");
        // Don't want to stop the actor for this.
        return Ok(buf);
    }

    // Then send the buffer as a single packet.
    if buf.len() > MAX_PACKET_SIZE {
        let len = buf.len();
        warn!(
            "message too large (for {target}): (serialised) message size {len}, max is {MAX_PACKET_SIZE}",
        );
        // Don't want to stop the actor for this.
        return Ok(buf);
    }
    let (buf, bytes_send) = socket.send_to(buf, target).await?;
    if bytes_send == buf.len() {
        Ok(buf)
    } else {
        Err(io::ErrorKind::WriteZero.into())
    }
}

/// Routes a message in `buf` using `router`.
///
/// Returns an error if the message can't be routed. Errors from deserialising
/// the message in `buf` are only logged using `warn!`.
async fn route_message<S, R, M>(router: &mut R, buf: &[u8], source: SocketAddr) -> io::Result<()>
where
    S: Serde,
    R: Route<M>,
    M: DeserializeOwned,
{
    match S::from_slice::<Message<M>>(buf) {
        Ok(msg) => match router.route(msg.msg, source).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let msg = format!("failed to route message (from {source}): {err}");
                Err(io::Error::new(io::ErrorKind::Other, msg))
            }
        },
        Err(err) => {
            warn!("error deserialising message (from {source}): {err}");
            // Don't want to stop the relay actor over this.
            Ok(())
        }
    }
}
