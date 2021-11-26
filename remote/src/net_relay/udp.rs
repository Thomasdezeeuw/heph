//! Module with the UDP implementation of the net relay.

use std::convert::TryFrom;
use std::io;
use std::net::SocketAddr;

use heph::actor::messages::Terminate;
use heph::actor::{self, NoMessages};
use heph::net::UdpSocket;
use heph::rt::{self, Signal};
use heph::util::either;
use log::warn;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::net_relay::{Route, Serde};

const MAX_PACKET_SIZE: usize = 1 << 16; // ~65kb.

/// Message type used for network relays.
pub(crate) enum RelayMessage<M> {
    /// Relay message `M` to `target`.
    Relay { message: M, target: SocketAddr },
    /// Stop the relay.
    Terminate,
}

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

/// Actor that handles remote messages.
///
/// It receives `Out`going messages from it's inbox and sends them to a remote
/// actor using UDP. Any `In`coming message on the same socket will be routed
/// using the `R`outer.
pub(crate) async fn remote_relay<S, Out, In, R, RT>(
    mut ctx: actor::Context<RelayMessage<Out>, RT>,
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

    loop {
        buf.clear();
        match either(ctx.receive_next(), socket.recv_from(&mut buf)).await {
            Ok(Ok(msg)) => match msg {
                // Received an outgoing message we want to relay to a remote
                // actor.
                RelayMessage::Relay { message, target } => {
                    send_message::<S, Out>(&mut socket, &mut buf, target, &message).await?
                }
                RelayMessage::Terminate => return Ok(()),
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

/// Send a `msg` to a remote actor at `target` address, using `socket`.
async fn send_message<S, M>(
    socket: &mut UdpSocket,
    buf: &mut Vec<u8>,
    target: SocketAddr,
    msg: &M,
) -> io::Result<()>
where
    S: Serde,
    M: Serialize,
{
    // Serialise the message to our buffer first.
    if let Err(err) = S::to_buf(&mut *buf, msg) {
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
    match S::from_slice(buf) {
        Ok(msg) => match router.route(msg, source).await {
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
