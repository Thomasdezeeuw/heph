//! Module with the TCP implementation of the net relay.

use std::io;
use std::net::SocketAddr;
use std::pin::pin;

use heph::actor::{self, NoMessages};
use heph_rt as rt;
use heph_rt::net::TcpStream;
use heph_rt::util::either;
use log::warn;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::net_relay::uuid::UuidGenerator;
use crate::net_relay::{DeIter, Message, Route, Serde};

const INITIAL_BUF_SIZE: usize = 1 << 12; // 4kb.

/// Message type used for network relays using TCP.
#[derive(Debug)]
pub enum RelayMessage<M> {
    /// Relay the message `M`.
    Relay(M),
    /// Stop the relay.
    Terminate,
}

impl<M> From<M> for RelayMessage<M> {
    fn from(msg: M) -> RelayMessage<M> {
        RelayMessage::Relay(msg)
    }
}

/* TODO: add these once specialization is possible.
impl<M> From<Terminate> for RelayMessage<M> {
    fn from(_: Terminate) -> RelayMessage<M> {
        RelayMessage::Terminate
    }
}

impl<M> TryFrom<Signal> for RelayMessage<M> {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(RelayMessage::Terminate),
            _ => Err(()),
        }
    }
}
*/

/// Actor that handles remote messages.
///
/// It receives `Out`going messages from it's inbox and sends them to a remote
/// actor using TCP. Any `In`coming message on the same socket will be routed
/// using the `R`outer.
pub(crate) async fn remote_relay<S, Out, In, R, RT>(
    mut ctx: actor::Context<RelayMessage<Out>, RT>,
    remote_address: SocketAddr,
    mut router: R,
) -> io::Result<()>
where
    S: Serde,
    Out: Serialize,
    In: DeserializeOwned,
    RT: rt::Access,
    R: Route<In>,
{
    let stream = TcpStream::connect(ctx.runtime_ref(), remote_address).await?;
    stream.set_nodelay(true)?;

    let mut uuid_gen = UuidGenerator::new();
    let mut send_buf = Vec::with_capacity(INITIAL_BUF_SIZE);

    let mut recv_data = pin!(stream.recv(Vec::with_capacity(INITIAL_BUF_SIZE)));
    loop {
        match either(ctx.receive_next(), recv_data.as_mut()).await {
            // Received an outgoing message we want to relay to a remote actor.
            Ok(Ok(RelayMessage::Relay(msg))) => {
                send_buf = send_message::<S, Out>(&stream, send_buf, &mut uuid_gen, &msg).await?;
                send_buf.clear();
            }
            Ok(Ok(RelayMessage::Terminate) | Err(NoMessages)) => return Ok(()),
            // Received some incoming data.
            Err(Ok(mut buf)) => {
                route_messages::<S, R, In>(&mut router, &mut buf, remote_address).await?;
                recv_data.set(stream.recv(buf));
            }
            // Error receiving data.
            Err(Err(err)) => return Err(err),
        }
    }
}

/// Send a `msg` to the remote actor, using `stream`.
async fn send_message<S, M>(
    stream: &TcpStream,
    mut buf: Vec<u8>,
    uuid_gen: &mut UuidGenerator,
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
        warn!("error serialising message: {err}");
        // Don't want to stop the actor for this.
        return Ok(buf);
    }

    stream.send_all(buf).await
}

/// Routes all messages in `buf` using `router`.
///
/// Returns an error if the message can't be routed or can't be deserialised.
async fn route_messages<S, R, M>(
    router: &mut R,
    buf: &mut Vec<u8>,
    source: SocketAddr,
) -> io::Result<()>
where
    S: Serde,
    R: Route<M>,
    M: DeserializeOwned,
{
    let mut deserialiser = S::iter(&*buf);
    loop {
        match deserialiser.next() {
            Some(Ok(msg)) => match router.route(msg, source).await {
                Ok(()) => continue,
                Err(err) => {
                    let msg = format!("failed to route message: {err}");
                    return Err(io::Error::new(io::ErrorKind::Other, msg));
                }
            },
            Some(Err(err)) => {
                let msg = format!("failed to deserialise message: {err}");
                return Err(io::Error::new(io::ErrorKind::InvalidData, msg));
            }
            None => break,
        }
    }

    let n = deserialiser.byte_offset();
    drop(deserialiser);
    if n == buf.len() {
        buf.clear();
    } else {
        drop(buf.drain(..n));
    }
    Ok(())
}
