//! Network related types.
//!
//! The network module support two types of protocols:
//!
//! * [Transmission Control Protocol] (TCP) module provides three main types:
//!   * A [TCP stream] between a local and a remote socket.
//!   * A [TCP listening socket], a socket used to listen for connections.
//!   * A [TCP server], listens for connections and starts a new actor for each.
//! * [User Datagram Protocol] (UDP) only provides a single socket type:
//!   * [`UdpSocket`].
//!
//! This module also has the [`connect`] function which is used to connect to
//! actors on remote nodes.
//!
//! [Transmission Control Protocol]: crate::net::tcp
//! [TCP stream]: crate::net::TcpStream
//! [TCP listening socket]: crate::net::TcpListener
//! [TCP server]: crate::net::tcp::Server
//! [User Datagram Protocol]: crate::net::udp
//! [`UdpSocket`]: crate::net::UdpSocket

use std::io;
use std::net::SocketAddr;

use log::{debug, warn};
use serde::Serialize;

use crate::actor::context::ThreadSafe;
use crate::actor::{self, Actor, NewActor};
use crate::actor_ref::ActorRef;
use crate::rt::options::{ActorOptions, Priority};
use crate::rt::RuntimeRef;
use crate::supervisor::{Supervisor, SupervisorStrategy};

/// A macro to try an I/O function.
///
/// Note that this is used in the tcp and udp modules and has to be defined
/// before them, otherwise this would have been place below.
macro_rules! try_io {
    ($op:expr) => {
        loop {
            match $op {
                Ok(ok) => break Poll::Ready(Ok(ok)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    };
}

pub mod tcp;
pub mod udp;

#[doc(no_inline)]
pub use tcp::{TcpListener, TcpStream};
#[doc(no_inline)]
pub use udp::UdpSocket;

#[cfg(test)]
mod tests;

/// Connect to an actor, [registered] with `name`, on a remote node at
/// `address`.
// TODO: doc remote actor references.
// * Max size.
pub fn connect(
    runtime_ref: &mut RuntimeRef,
    address: SocketAddr,
    name: &'static str,
) -> ActorRef<Remote> {
    // TODO: ensure 1 actor per 1 ip. Or don't do this...
    // Use some kind of hashmap in the shard data of the runtime for this.

    let supervisor = RelaySupervisor(address, 0);
    #[allow(trivial_casts)]
    let relay_actor = relay_actor as fn(_, _) -> _;
    let options = ActorOptions::default()
        .with_priority(Priority::HIGH)
        .mark_ready();
    let actor_ref = runtime_ref.spawn(supervisor, relay_actor, address, options);
    actor_ref.to_named(name).unwrap_or_else(|| unreachable!())
}

/// Message type used to indicate the [`ActorRef`] sends a message to an actor
/// on a remote node.
///
/// Because its not possible to determine the message type of the remote actor
/// (in the context of Rust's type system) this type is a catch-all for remote
/// actor references, effectively making it untyped.
///
/// To create a remote actor reference see [`connect`].
///
/// # Notes
///
/// This type implements [`From`]`<M: `[`Serialize`]`>` allowing any type to be
/// send using [`ActorRef::send`], including incorrect message types. However
/// sending an incorrect message (type) does not return an [`SendError`] as only
/// the remote actor can determine if the message type is correct or not.
///
/// [`ActorRef`]: crate::actor_ref::ActorRef
/// [`Serialize`]: serde::Serialize
/// [`ActorRef::send`]: crate::actor_ref::ActorRef::send
/// [`SendError`]:crate::actor_ref::SendError
#[allow(missing_debug_implementations)]
pub struct Remote {
    data: Box<dyn erased_serde::Serialize + Send + Sync>,
}

impl<M> From<M> for Remote
where
    M: Serialize + Send + Sync + 'static,
{
    fn from(msg: M) -> Remote {
        Remote {
            data: Box::from(msg),
        }
    }
}

/// [`Supervisor`] for the [`relay_actor`] actor.
struct RelaySupervisor(SocketAddr, usize);

/// Maximum number of times the [`relay_actor`] actor can fail before stopping
/// it.
const MAX_FAILS: usize = 3;

impl<NA, A> Supervisor<NA> for RelaySupervisor
where
    NA: NewActor<Argument = SocketAddr, Error = !, Actor = A>,
    A: Actor<Error = io::Error>,
{
    fn decide(&mut self, err: io::Error) -> SupervisorStrategy<NA::Argument> {
        warn!("relay actor for remote ActorRefs failed: error={}", err);
        self.1 += 1;
        if self.1 >= MAX_FAILS {
            SupervisorStrategy::Restart(self.0)
        } else {
            SupervisorStrategy::Stop
        }
    }

    fn decide_on_restart_error(&mut self, _: !) -> SupervisorStrategy<NA::Argument> {
        unreachable!()
    }

    fn second_restart_error(&mut self, _: !) {
        unreachable!()
    }
}

/// A conservative estimate for the maximum size of the data inside a UDP
/// packet.
const MAX_UDP_PACKET_SIZE: usize = 65000;

/// Actor that relay `Remote` messages to actors on the `remote` node.
async fn relay_actor(
    mut ctx: actor::Context<(&'static str, Remote), ThreadSafe>,
    remote: SocketAddr,
) -> io::Result<()> {
    let local = any_local_address(&remote);
    debug!(
        "connecting relay actor: local_addr={}, remote_addr={}",
        local, remote
    );
    let mut socket = UdpSocket::bind(&mut ctx, local).and_then(|socket| socket.connect(remote))?;

    let mut id = 0;
    let mut buf = Vec::new();
    loop {
        // TODO: read from socket to support RPC.
        // FIXME: This never stops even if we get no more messages.
        let (actor, msg) = ctx.receive_next().await;
        let msg = RemoteMessage {
            id: next_id(&mut id),
            actor,
            data: msg.data,
        };

        buf.clear();
        serde_json::to_writer(&mut buf, &msg)?;
        if buf.len() > MAX_UDP_PACKET_SIZE {
            warn!("can't send message to remote actor, message too large: remote_actor={} remote_address={} message_size={}", actor, remote, buf.len());
            continue;
        }

        match socket.send(&buf).await {
            Ok(send_size) if send_size < buf.len() => {
                warn!(
                    "failed to send entire message to remote actor: bytes_send={} message_size={}",
                    send_size,
                    buf.len()
                );
            }
            Ok(_) => {}
            Err(err) => {
                warn!("failed to send message to remote actor: {}", err);
                return Err(err);
            }
        }
    }
}

/// Returns a local address with the port set 0, allowing it to bind to any
/// port.
fn any_local_address(addr: &SocketAddr) -> SocketAddr {
    use std::net::{Ipv4Addr, Ipv6Addr};
    match addr {
        SocketAddr::V4(_) => (Ipv4Addr::LOCALHOST, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::LOCALHOST, 0).into(),
    }
}

/// Returns the next id for `id`.
fn next_id(id: &mut usize) -> usize {
    let i = *id;
    *id += 1;
    i
}

/// Message send to the remote node via UDP.
#[derive(Serialize)]
struct RemoteMessage {
    /// Unique id for the message used to respond to the message.
    id: usize,
    /// Actor to send the message to.
    actor: &'static str,
    /// Data that makes up the message to the actor.
    data: Box<dyn erased_serde::Serialize + Send + Sync>,
}
