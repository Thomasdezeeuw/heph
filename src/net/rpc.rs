//! Module with remote registry.
//!
//! Main types:
//! * [`RemoteRegistry`]: allow actors to be registered to receive message from
//!   actors on remote nodes.
//! * [`RemoteActors`]: creates a connection to a remote node and allow the
//!   creation of actor references to actors on that remote node.
//!
//! # Examples
//!
//! ```
//! // TODO.
//! ```
//!
//! Also see example 5 (a & b) in the examples directory.

// TODO: better name that `rpc` for this module, conflicts with `ActorRef::rpc`.
// TODO: support `ActorRef::rpc`.

use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{fmt, io};

use log::{debug, trace, warn};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::de::SliceRead;

use crate::actor::context::ThreadSafe;
use crate::actor::messages::Terminate;
use crate::actor::{self, Actor, NewActor};
use crate::actor_ref::ActorRef;
use crate::net::udp::{Connected, UdpSocket};
use crate::rt::options::{ActorOptions, Priority};
use crate::rt::{Runtime, Signal};
use crate::supervisor::{Supervisor, SupervisorStrategy};

/// Maximum number of times the actors can fail before stopping it.
const MAX_FAILS: usize = 3;

/// Type used for registering actors.
type Registrations = HashMap<&'static str, UntypedActorRef>;

/// Remote actor registry.
///
/// This type allow actors to be registered and be referenced by actors on
/// different nodes. See [`RemoteActors`] to connect to actors registered using
/// this type.
#[derive(Debug)]
#[must_use = "`RemoteRegistry::spawn_listener` must be called to ensure remote message are received"]
pub struct RemoteRegistry {
    registrations: Registrations,
}

impl RemoteRegistry {
    /// Create a new `RemoteRegistry`.
    pub fn new() -> RemoteRegistry {
        RemoteRegistry {
            registrations: HashMap::new(),
        }
    }

    /// Register an actor with `name`.
    ///
    /// Creating a [remote connection] to this node and then calling
    /// [`create_ref`] using the same `name` will create a remote actor
    /// reference that send messages to the actor registered here.
    ///
    /// Returns an error if another actor is already registered with the same
    /// name.
    ///
    /// [remote connection]: RemoteActors
    /// [`create_ref`]: RemoteActors::create_ref
    pub fn register<M>(
        &mut self,
        name: &'static str,
        actor_ref: ActorRef<M>,
    ) -> Result<(), AlreadyRegistered>
    where
        M: DeserializeOwned + Send + Sync + 'static,
    {
        use std::collections::hash_map::Entry;
        match self.registrations.entry(name) {
            Entry::Vacant(entry) => {
                drop(entry.insert(actor_ref.into()));
                Ok(())
            }
            Entry::Occupied(_) => Err(AlreadyRegistered { name }),
        }
    }

    // TODO: add a method that uses `ActorGroup`, allow multiple actor to
    // receive the same message?

    /// De-register the actor previously registered with `name`.
    ///
    /// Returns an error if no actor was previously registered using `name`.
    pub fn deregister(&mut self, name: &'static str) -> Result<(), ()> {
        if let Some(_) = self.registrations.remove(name) {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Spawns an actor that listens for incoming messages on `address` and
    /// relays the messages to the actors registered in this registry.
    pub fn spawn_listener(
        self,
        runtime: &mut Runtime,
        address: SocketAddr,
    ) -> ActorRef<RegistryMessage> {
        let registrations = Arc::new(self.registrations);
        let supervisor = RegistrySupervisor(address, registrations.clone(), 0);
        #[allow(trivial_casts)]
        let relay_listener = relay_listener as fn(_, _, _) -> _;
        let options = ActorOptions::default()
            .with_priority(Priority::HIGH)
            .mark_ready();
        let args = (address, registrations);
        runtime.spawn(supervisor, relay_listener, args, options)
    }
}

/// Error returned when an actor is already registered using the same name.
///
/// Returned by [`RemoteRegistry::register`].
#[derive(Debug)]
pub struct AlreadyRegistered {
    name: &'static str,
}

impl fmt::Display for AlreadyRegistered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "actor already registered using name '{}'", self.name)
    }
}

impl Error for AlreadyRegistered {}

/// `ActorRef` but without a type.
struct UntypedActorRef {
    inner: Box<dyn DeserializeSend + Send + Sync>,
}

/// Trait that allows [`UntypedActorRef`] to be untyped.
trait DeserializeSend {
    /// Deserialise a message from `deserialiser` and send it using the
    /// underlying actor reference.
    fn try_send(
        &self,
        deserialiser: &mut serde_json::Deserializer<SliceRead<'_>>,
    ) -> Result<(), SendError>;
}

impl<M> DeserializeSend for ActorRef<M>
where
    M: DeserializeOwned + Send + Sync,
{
    fn try_send(
        &self,
        deserialiser: &mut serde_json::Deserializer<SliceRead<'_>>,
    ) -> Result<(), SendError> {
        M::deserialize(deserialiser)
            .map_err(|_| SendError::FailedDeserialise)
            .and_then(|msg| self.send(msg).map_err(|_| SendError::FailedSend))
    }
}

impl UntypedActorRef {
    /// Attempts to deserialise the message in `buf` and send it to the actor.
    fn try_send(
        &self,
        deserialiser: &mut serde_json::Deserializer<SliceRead<'_>>,
    ) -> Result<(), SendError> {
        self.inner.try_send(deserialiser)
    }
}

impl<M> From<ActorRef<M>> for UntypedActorRef
where
    M: DeserializeOwned + Send + Sync + 'static,
{
    fn from(actor_ref: ActorRef<M>) -> UntypedActorRef {
        UntypedActorRef {
            inner: Box::new(actor_ref),
        }
    }
}

impl fmt::Debug for UntypedActorRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UntypedActorRef")
    }
}

/// Error returned by [`UntypedActorRef::try_send`].
enum SendError {
    /// Failed to deserialise the message.
    FailedDeserialise,
    /// Failed to send the message.
    FailedSend,
}

/// [`Supervisor`] for the [`relay_actor`] actor.
struct RegistrySupervisor(SocketAddr, Arc<Registrations>, usize);

impl<NA, A> Supervisor<NA> for RegistrySupervisor
where
    NA: NewActor<Argument = (SocketAddr, Arc<Registrations>), Error = !, Actor = A>,
    A: Actor<Error = io::Error>,
{
    fn decide(&mut self, err: io::Error) -> SupervisorStrategy<NA::Argument> {
        warn!("relay listener actor failed: error={}", err);
        self.2 += 1;
        if self.2 >= MAX_FAILS {
            SupervisorStrategy::Restart((self.0, self.1.clone()))
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

/// Message send to the listener actor started by
/// [`RemoteRegistry::spawn_listener`].
///
/// Can be used to stop the listener.
#[derive(Debug)]
pub struct RegistryMessage {
    _priv: (),
}

impl From<Terminate> for RegistryMessage {
    fn from(_: Terminate) -> RegistryMessage {
        RegistryMessage { _priv: () }
    }
}

impl TryFrom<Signal> for RegistryMessage {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => {
                Ok(RegistryMessage { _priv: () })
            }
        }
    }
}

/// Actor that relays messages from peers to local actors.
async fn relay_listener(
    mut ctx: actor::Context<RegistryMessage, ThreadSafe>,
    local: SocketAddr,
    registrations: Arc<Registrations>,
) -> io::Result<()> {
    debug!("listener for remote messages: local_addr={}", local);
    let mut socket = UdpSocket::bind(&mut ctx, local)?;

    let mut buf = vec![0; u16::MAX as usize];
    loop {
        // FIXME: read message incoming packets for RPC.

        // Read a single message from the socket.
        buf.resize(buf.capacity(), 0);
        let (n, remote) = socket.recv_from(&mut buf).await?;
        buf.truncate(n);
        trace!("got packet from remote actor: packet_size={}", n);

        // Parse the metadata.
        let mut deserialiser = serde_json::Deserializer::from_slice(&buf);
        let meta: MessageMetadata<'_> = match Deserialize::deserialize(&mut deserialiser) {
            Ok(meta) => meta,
            Err(err) => {
                warn!(
                    "failed to parse message metadata from remote node: remote_addr={} error=\"{}\"",
                    remote, err
                );
                continue;
            }
        };

        debug!(
            "parsed remote message metadata: id={}, actor={}",
            meta.id, meta.actor
        );

        // Lookup the local actor.
        if let Some(actor_ref) = registrations.get(meta.actor) {
            // Try to send the message to actor.
            match actor_ref.try_send(&mut deserialiser) {
                Ok(()) => {}
                Err(SendError::FailedDeserialise) => {
                    warn!(
                        "failed to parse message from remote node: remote_addr={} id={} local_actor={}",
                        remote,
                        meta.id,
                        meta.actor,
                    );
                }
                Err(SendError::FailedSend) => {
                    warn!(
                        "failed to relay remote message to local actor: remote_addr={} id={} local_actor={}",
                        remote,
                        meta.id,
                        meta.actor,
                    );
                }
            }
        } else {
            warn!(
                "no actor registered to receive message: remote_addr={} id={} local_actor={}",
                remote, meta.id, meta.actor,
            );
        }
    }
}

/// Connection to a remote node.
///
/// This type allows for connection to one or more actors on the same remote
/// node. To connect to a remote actor it is required to register the actor
/// first using [`RemoteRegistry`] (on the remote node).
///
/// See the [module level documentation] for examples.
///
/// [module level documentation]: crate::net::rpc
///
/// # Notes
///
/// The messages are send using UDP. This has two limitations:
/// * The size of a single message is limited to roughly 65,000 bytes.
/// * Delivery of the message is not guaranteed (but the same is true for any
///   actor reference).
#[derive(Debug)]
pub struct RemoteActors {
    actor_ref: ActorRef<(&'static str, Remote)>,
}

impl RemoteActors {
    /// Create a new connection to a remote node.
    pub fn connect(runtime: &mut Runtime, address: SocketAddr) -> RemoteActors {
        let supervisor = MsgRelaySupervisor(address, 0);
        #[allow(trivial_casts)]
        let msg_relay = msg_relay as fn(_, _) -> _;
        let options = ActorOptions::default()
            .with_priority(Priority::HIGH)
            .mark_ready();
        let actor_ref = runtime.spawn(supervisor, msg_relay, address, options);
        RemoteActors { actor_ref }
    }

    /// Create a reference to the actor [registered] with `name` on the remote
    /// node this is connected to.
    ///
    /// [registered]: RemoteRegistry::register
    pub fn create_ref(&self, name: &'static str) -> ActorRef<Remote> {
        self.actor_ref
            .clone()
            .to_named(name)
            .unwrap_or_else(|| unreachable!())
    }
}

/// Message type used to indicate the [`ActorRef`] sends a message to an actor
/// on a remote node.
///
/// Because its not possible to determine the message type of the remote actor
/// (in the context of Rust's type system) this type is a catch-all for remote
/// actor references, effectively making it untyped.
///
/// To create a remote actor reference see [`RemoteActors`].
///
/// # Notes
///
/// This type implements [`From`]`<M: `[`Serialize`]`>` allowing any type that
/// implements [`Serialize`] to be send using [`ActorRef::send`], including
/// incorrect message types. Sending an incorrect message (type) does
/// not return an [`SendError`] as only the remote actor can determine if the
/// message type is correct or not.
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

/// [`Supervisor`] for the [`msg_relay`] actor.
struct MsgRelaySupervisor(SocketAddr, usize);

impl<NA, A> Supervisor<NA> for MsgRelaySupervisor
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

/// Actor that relays `Remote` messages to actors on the `remote` node.
async fn msg_relay(
    mut ctx: actor::Context<(&'static str, Remote), ThreadSafe>,
    remote: SocketAddr,
) -> io::Result<()> {
    let (mut socket, local) = new_socket(&mut ctx, remote)?;

    let mut id = 0;
    let mut buf = Vec::new();
    loop {
        // TODO: read from socket to support RPC.
        // FIXME: This never stops even if we get no more messages.
        let (actor, msg) = ctx.receive_next().await;
        let meta = MessageMetadata {
            id: next_id(&mut id),
            actor,
        };

        buf.clear();
        serde_json::to_writer(&mut buf, &meta)?;
        serde_json::to_writer(&mut buf, &msg.data)?;
        if buf.len() > MAX_UDP_PACKET_SIZE {
            warn!("can't send message to remote actor, message too large: remote_actor={} remote_address={} message_size={}", actor, remote, buf.len());
            continue;
        }

        debug!(
            "sending message to remote actor: local_addr={}, remote_addr={}, actor={}, message_size={}",
            local, remote, actor, buf.len(),
        );
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

/// Create a new socket connected to `remote`, bound to any local address.
fn new_socket<M>(
    ctx: &mut actor::Context<M, ThreadSafe>,
    remote: SocketAddr,
) -> io::Result<(UdpSocket<Connected>, SocketAddr)> {
    let local = any_local_address(&remote);
    UdpSocket::bind(ctx, local).and_then(|mut socket| {
        let local = socket.local_addr()?;
        debug!(
            "connecting remote actor message relay: local_addr={}, remote_addr={}",
            local, remote
        );
        socket.connect(remote).map(|socket| (socket, local))
    })
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

/// Metadata for the message send to the remote node via UDP.
#[derive(Deserialize, Serialize)]
struct MessageMetadata<'a> {
    /// Unique id for the message used to respond to the message.
    id: usize,
    /// Actor to send the message to.
    actor: &'a str,
}
