//! Module with the UDP implementation of the net relay.

use std::convert::TryFrom;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::{fmt, io};

use heph::actor::messages::Terminate;
use heph::actor::{self, NoMessages};
use heph::net::UdpSocket;
use heph::rt::{self, Signal};
use heph::util::either;
use log::warn;
use serde::de::{self, Deserialize, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeStruct, Serializer};

use crate::net_relay::uuid::{Uuid, UuidGenerator};
use crate::net_relay::{Route, Serde};

const MAX_PACKET_SIZE: usize = 1 << 16; // ~65kb.

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
    Relay { message: M, target: SocketAddr },
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
    let mut socket = UdpSocket::bind(&mut ctx, local_address)?;
    let mut buf = Vec::with_capacity(MAX_PACKET_SIZE);
    let mut uuid_gen = UuidGenerator::new();

    loop {
        buf.clear();
        match either(ctx.receive_next(), socket.recv_from(&mut buf)).await {
            Ok(Ok(msg)) => match msg {
                // Received an outgoing message we want to relay to a remote
                // actor.
                UdpRelayMessage::Relay { message, target } => {
                    send_message::<S, Out>(&mut socket, &mut buf, &mut uuid_gen, target, &message)
                        .await?
                }
                UdpRelayMessage::Terminate => return Ok(()),
            },
            // TODO: do we want to continue here? Still relaying messages from
            // remote actors.
            // No more messages.
            Ok(Err(NoMessages)) => return Ok(()),
            // Received an incoming packet.
            Err(Ok((_, source))) => route_message::<S, R, In>(&mut router, &buf, source).await?,
            // Error receiving a packet.
            Err(Err(err)) => return Err(err),
        }
    }
}

/// Message type that wraps the user's message `M`.
struct Message<M> {
    uuid: Uuid,
    msg: M,
}

// NOTE: manually implementing this instead of deriving to not pull in a bunch
// of dependencies.
impl<'de, M> Deserialize<'de> for Message<M>
where
    M: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Uuid,
            Msg,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`uuid` or `message`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "uuid" => Ok(Field::Uuid),
                            "message" => Ok(Field::Msg),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct MessageVisitor<M>(PhantomData<M>);

        impl<'de, M> Visitor<'de> for MessageVisitor<M>
        where
            M: Deserialize<'de>,
        {
            type Value = Message<M>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct Message")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Message<M>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut uuid = None;
                let mut msg = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Uuid => {
                            if uuid.is_some() {
                                return Err(de::Error::duplicate_field("uuid"));
                            }
                            uuid = Some(map.next_value()?);
                        }
                        Field::Msg => {
                            if msg.is_some() {
                                return Err(de::Error::duplicate_field("message"));
                            }
                            msg = Some(map.next_value()?);
                        }
                    }
                }
                let uuid = uuid.ok_or_else(|| de::Error::missing_field("uuid"))?;
                let msg = msg.ok_or_else(|| de::Error::missing_field("message"))?;
                Ok(Message { uuid, msg })
            }
        }

        const FIELDS: &'static [&'static str] = &["uuid", "message"];
        deserializer.deserialize_struct("Message", FIELDS, MessageVisitor(PhantomData))
    }
}

impl<M> Serialize for Message<M>
where
    M: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Message", 2)?;
        state.serialize_field("uuid", &self.uuid)?;
        state.serialize_field("message", &self.msg)?;
        state.end()
    }
}

/// Send a `msg` to a remote actor at `target` address, using `socket`.
async fn send_message<S, M>(
    socket: &mut UdpSocket,
    buf: &mut Vec<u8>,
    uuid_gen: &mut UuidGenerator,
    target: SocketAddr,
    msg: &M,
) -> io::Result<()>
where
    S: Serde,
    M: Serialize,
{
    // Serialise the message to our buffer first.
    let uuid = uuid_gen.next();
    let msg = Message { uuid, msg };
    if let Err(err) = S::to_buf(&mut *buf, &msg) {
        warn!("error serialising message (for {}): {}", target, err);
        // Don't want to stop the actor for this.
        return Ok(());
    }

    // Then send the buffer as a single packet.
    if buf.len() > MAX_PACKET_SIZE {
        warn!(
            "message too large (for {}): (serialised) message size {}, max is {}",
            target,
            buf.len(),
            MAX_PACKET_SIZE,
        );
        // Don't want to stop the actor for this.
        return Ok(());
    }
    socket.send_to(buf, target).await.and_then(|bytes_send| {
        if bytes_send == buf.len() {
            Ok(())
        } else {
            Err(io::ErrorKind::WriteZero.into())
        }
    })
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
                let msg = format!("failed to route message (from {}): {}", source, err);
                return Err(io::Error::new(io::ErrorKind::Other, msg));
            }
        },
        Err(err) => {
            warn!("error deserialising message (from {}): {}", source, err);
            // Don't want to stop the relay actor over this.
            Ok(())
        }
    }
}
