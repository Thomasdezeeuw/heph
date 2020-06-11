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

use futures_util::future::{select, Either};
use log::{debug, trace, warn};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::de::SliceRead;

use crate::actor;
use crate::actor::context::ThreadSafe;
use crate::actor::messages::Terminate;
use crate::actor_ref::{ActorRef, RpcMessage, RpcResponse};
use crate::net::udp::{Connected, UdpSocket};
use crate::rt::options::{ActorOptions, Priority};
use crate::rt::{Runtime, Signal};
use crate::supervisor::RestartSupervisor;

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
        let args = (address, registrations);
        let supervisor = RestartSupervisor::new("relay_listener", args.clone());
        #[allow(trivial_casts)]
        let relay_listener = relay_listener as fn(_, _, _) -> _;
        let options = ActorOptions::default()
            .with_priority(Priority::HIGH)
            .mark_ready();
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

    let mut buf = Vec::with_capacity(MAX_UDP_PACKET_SIZE);
    loop {
        // FIXME: read message incoming packets for RPC.

        // Read a single message from the socket.
        debug_assert!(buf.capacity() >= MAX_UDP_PACKET_SIZE);
        unsafe { buf.set_len(MAX_UDP_PACKET_SIZE) };
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

        if meta.rpc {
            // TODO.
            todo!("TODO: handle RPC messages");
        }

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
        let supervisor = RestartSupervisor::new("remote_actor_ref", address);
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
            .into_named(name)
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
    inner: RemoteInner,
}

enum RemoteInner {
    /// Single message.
    Message(Box<dyn erased_serde::Serialize + Send + Sync>),
    /// Remote produce call, effectively an untyped [`RpcMessage`].
    Rpc {
        request: Box<dyn erased_serde::Serialize + Send + Sync>,
        response: Box<dyn DeserializeRpcResponse + Send + Sync>,
    },
}

impl<M> From<M> for Remote
where
    M: Serialize + Send + Sync + 'static,
{
    fn from(msg: M) -> Remote {
        Remote {
            inner: RemoteInner::Message(Box::from(msg)),
        }
    }
}

impl<Req, Res> From<RpcMessage<Req, Res>> for Remote
where
    Req: Serialize + Send + Sync + 'static,
    Res: DeserializeOwned + Send + Sync + 'static,
{
    fn from(msg: RpcMessage<Req, Res>) -> Remote {
        Remote {
            inner: RemoteInner::Rpc {
                request: Box::from(msg.request),
                response: Box::from(msg.response),
            },
        }
    }
}

/// A conservative estimate for the maximum size of the data inside a UDP
/// packet.
const MAX_MESSAGE_SIZE: usize = 65000;
const MAX_UDP_PACKET_SIZE: usize = 1 << 16;

/// Actor that relays `Remote` messages to actors on the `remote` node.
async fn msg_relay(
    mut ctx: actor::Context<(&'static str, Remote), ThreadSafe>,
    remote: SocketAddr,
) -> io::Result<()> {
    let mut socket = RelaySocket::new(&mut ctx, remote)?;

    // FIXME: This never stops.
    loop {
        debug_assert!(socket.buf.capacity() >= MAX_UDP_PACKET_SIZE);
        unsafe { socket.buf.set_len(MAX_UDP_PACKET_SIZE) };

        match select(socket.socket.recv(&mut socket.buf), ctx.receive_next()).await {
            Either::Left((res, _)) => {
                // Read a packet.
                socket.buf.truncate(res?);
                socket.relay_rpc_response();
            }
            Either::Right(((actor, msg), _)) => {
                // Received a message to relay.
                socket.send_message(actor, msg).await?;
            }
        }
    }
}

/// Wrapper around [`UdpSocket`] to send [`Remote`] messages.
struct RelaySocket {
    socket: UdpSocket<Connected>,
    local: SocketAddr,
    remote: SocketAddr,
    /// **Capacity must be at least MAX_UDP_PACKET_SIZE bytes!**
    buf: Vec<u8>,
    /// Id for the messages.
    id: usize,
    /// Map for [`RpcResponse`s].
    responses: HashMap<usize, Box<dyn DeserializeRpcResponse + Send + Sync>>,
}

impl RelaySocket {
    /// Create a new socket connected to `remote`, bound to any local address.
    fn new<M>(
        ctx: &mut actor::Context<M, ThreadSafe>,
        remote: SocketAddr,
    ) -> io::Result<RelaySocket> {
        let local = any_local_address(&remote);
        UdpSocket::bind(ctx, local).and_then(|mut socket| {
            let local = socket.local_addr()?;
            debug!(
                "connecting remote actor message relay: local_addr={}, remote_addr={}",
                local, remote
            );
            socket.connect(remote).map(|socket| RelaySocket {
                socket,
                local,
                remote,
                buf: Vec::with_capacity(MAX_UDP_PACKET_SIZE),
                id: 0,
                responses: HashMap::new(),
            })
        })
    }

    /// Returns the next id.
    fn next_id(&mut self) -> usize {
        let i = self.id;
        self.id += 1;
        i
    }

    /// Send `msg` to remote `actor`.
    async fn send_message(&mut self, actor: &'static str, msg: Remote) -> io::Result<()> {
        let id = self.next_id();
        let (msg, rpc) = match msg.inner {
            RemoteInner::Message(msg) => (msg, false),
            RemoteInner::Rpc { request, response } => {
                let r = self.responses.insert(id, response);
                debug_assert!(r.is_none());
                (request, true)
            }
        };

        let meta = MessageMetadata { id, actor, rpc };

        // Write the message metadata and the message itself.
        self.buf.clear();
        serde_json::to_writer(&mut self.buf, &meta)?;
        serde_json::to_writer(&mut self.buf, &msg)?;

        // We don't (yet) support splitting a single message over multiple UDP
        // pakcets.
        if self.buf.len() > MAX_MESSAGE_SIZE {
            warn!("can't send message to remote actor, message too large: local_addr={} remote_addr={} actor={} message_size={}",
                self.local,
                self.remote,
                actor,
                self.buf.len());
            return Ok(());
        }

        debug!(
            "sending message to remote actor: local_addr={} remote_addr={} actor={} message_size={}",
            self.local,
            self.remote,
            actor,
            self.buf.len(),
        );
        match self.socket.send(&self.buf).await {
            Ok(send_size) if send_size < self.buf.len() => {
                warn!(
                    "failed to send entire message to remote actor: local_addr={} remote_addr={} actor={} bytes_send={} message_size={}",
                    self.local,
                    self.remote,
                    actor,
                    send_size,
                    self.buf.len()
                );
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(err) => {
                warn!(
                    "failed to send message to remote actor: local_addr={} remote_addr={} actor={} error={}",
                    self.local,
                    self.remote,
                    actor,
                    err,
                );
                Err(err)
            }
        }
    }

    /// Relay the message in `self.buf` to the correct [`RpcResponse`].
    fn relay_rpc_response(&mut self) {
        let mut deserialiser = serde_json::Deserializer::from_slice(&self.buf);
        let meta: RpcResponseMetadata = match Deserialize::deserialize(&mut deserialiser) {
            Ok(meta) => meta,
            Err(err) => {
                warn!(
                    "failed to parse RPC response metadata from remote node: remote_addr={} error=\"{}\"",
                    self.remote, err
                );
                return;
            }
        };
        // FIXME: possible attack vector by sending messages with the id, not
        // allow the actual actors to ever respond.
        match self.responses.remove(&meta.id) {
            Some(response) => match response.try_respond(&mut deserialiser) {
                Ok(()) => {}
                Err(SendError::FailedDeserialise) => {
                    warn!(
                        "failed to parse RPC response from remote node: remote_addr={} id={}",
                        self.remote, meta.id
                    );
                }
                Err(SendError::FailedSend) => {
                    warn!(
                        "failed to relay RPC response to local actor: remote_addr={} id={}",
                        self.remote, meta.id
                    );
                }
            },
            None => {
                warn!(
                    "got a RPC response with unknown id: remote_addr={} id={}",
                    self.remote, meta.id
                );
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

/// Trait to box `RpcResponse<Res>` with different types of `Res`.
trait DeserializeRpcResponse {
    /// Serialise the message in `deserialiser` and respond to the RPC.
    /// Deserialise a message from `deserialiser` and respond with it to the
    /// underlying [`RpcResponse`].
    fn try_respond(
        self: Box<Self>,
        deserialiser: &mut serde_json::Deserializer<SliceRead<'_>>,
    ) -> Result<(), SendError>;
}

impl<Res> DeserializeRpcResponse for RpcResponse<Res>
where
    Res: DeserializeOwned + Send + Sync + 'static,
{
    fn try_respond(
        self: Box<Self>,
        deserialiser: &mut serde_json::Deserializer<SliceRead<'_>>,
    ) -> Result<(), SendError> {
        Res::deserialize(deserialiser)
            .map_err(|_| SendError::FailedDeserialise)
            .and_then(|msg| self.respond(msg).map_err(|_| SendError::FailedSend))
    }
}

/// Metadata for the message send to the remote node via UDP.
#[derive(Deserialize, Serialize)]
struct MessageMetadata<'a> {
    /// Unique id for the message used to respond to the message.
    id: usize,
    /// Actor to send the message to.
    actor: &'a str,
    /// `true` if the sending actor expects a response.
    rpc: bool,
}

#[derive(Deserialize, Serialize)]
struct RpcResponseMetadata {
    /// Id of the RPC request (in [`MessageMetadata`]).
    id: usize,
}
