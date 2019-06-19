//! Module with the Actor Registry.

use std::any::TypeId;
use std::collections::HashMap;
use std::mem;

use crate::actor::NewActor;
use crate::actor_ref::{ActorRef, Local};

/// The actor registry holds local actor references based on the type of the
/// actor. This allows actors to be looked up based on there types, maintaining
/// type safety with the flexibility of dynamically looking up actors.
#[derive(Debug)]
pub struct ActorRegistry {
    /// The type signature below is not accurate. The actual stored type is
    /// `ActorRef<M, >`, where `M` is different based on each `TypeId`. But
    /// since `M` is different for each value we can't store in a `HashMap`
    /// without using tricks.
    ///
    /// Dropping the actor registry is also problematic, since don't know the
    /// type of each actor reference and when dropping is not possible to
    /// determine the type. So our only option is leaking the memory, which
    /// isn't that bad. Seeing how the actor system is likely shutting down
    /// (it's been dropped in any case) and the `MailBox` the reference points
    /// to is already dropped (just not deallocated) because we only store a
    /// weak reference.
    ///
    /// See the `into_value`, `from_value` and `from_value_ref` functions below
    /// for more details.
    inner: HashMap<TypeId, *mut ()>,
}

impl ActorRegistry {
    /// Create a new registry.
    pub fn new() -> ActorRegistry {
        ActorRegistry {
            inner: HashMap::new(),
        }
    }

    /// Register an actor.
    pub fn register<NA>(&mut self, actor_ref: ActorRef<Local<NA::Message>>) -> Option<ActorRef<Local<NA::Message>>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        let value = into_value::<NA>(actor_ref);
        self.inner.insert(key, value)
            .map(|actor_ref| unsafe { from_value::<NA>(actor_ref) })
    }

    /// Deregister an actor.
    pub fn deregister<NA>(&mut self) -> Option<ActorRef<Local<NA::Message>>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        self.inner.remove(&key)
            .map(|actor_ref| unsafe { from_value::<NA>(actor_ref) })
    }

    /// Lookup an actor.
    pub fn lookup<NA>(&mut self) -> Option<ActorRef<Local<NA::Message>>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        self.inner.get(&key)
            .map(|actor_ref| unsafe { from_value_ref::<NA>(actor_ref) })
    }
}

/// Convert a local actor reference into a value that can be inserted into
/// the hash map.
#[inline]
fn into_value<NA>(actor_ref: ActorRef<Local<NA::Message>>) -> *mut ()
    where NA: NewActor + 'static,
{
    // This is sort of safe. `ActorRef<Local<M>>` is represented as
    // `Weak<RefCell<MailBox<M>>>`, and any `Weak` type is just a pointer under
    // the hood. So this transmutes from a pointer to another pointer and should
    // be safe(-ish).
    unsafe { mem::transmute(actor_ref) }
}

/// Convert a value coming from the hash map into a typed actor reference. This
/// is the reverse of `into_value`.
///
/// # Unsafety
///
/// The caller needs to ensure the type is correct, **if the type is not correct
/// this will cause undefined behaviour**.
#[inline]
unsafe fn from_value<NA>(value: *mut ()) -> ActorRef<Local<NA::Message>>
    where NA: NewActor + 'static,
{
    // See `into_value` for comments about safety.
    mem::transmute(value)
}

/// Convert a value coming from the hash map into a cloned actor reference.
///
/// # Unsafety
///
/// The caller needs to ensure the type is correct, **if the type is not correct
/// this will cause undefined behaviour**.
#[inline]
#[allow(clippy::trivially_copy_pass_by_ref)]
unsafe fn from_value_ref<NA>(value_ref: &*mut ()) -> ActorRef<Local<NA::Message>>
    where NA: NewActor + 'static,
{
    // See `into_value` for comments about safety.
    #[allow(clippy::transmute_ptr_to_ptr)]
    let actor_ref_ref: &ActorRef<Local<NA::Message>> = mem::transmute(value_ref);
    actor_ref_ref.clone()
}
