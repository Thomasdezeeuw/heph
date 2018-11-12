//! Module with the Actor Registry.

use std::any::TypeId;
use std::collections::HashMap;
use std::mem;

use crate::actor::NewActor;
use crate::actor_ref::LocalActorRef;

/// The actor registry holds local actor references based on the type of the
/// actor. This allows actors to be looked up based on there types, maintaining
/// type safety with the flexibility of dynamically looking up actors.
#[derive(Debug)]
pub struct ActorRegistry {
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
    pub fn register<NA>(&mut self, actor_ref: LocalActorRef<NA::Message>) -> Option<LocalActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        let value = ActorRegistry::into_value::<NA>(actor_ref);
        self.inner.insert(key, value)
            .map(|actor_ref| ActorRegistry::from_value::<NA>(actor_ref))
    }

    /// Deregister an actor.
    pub fn deregister<NA>(&mut self) -> Option<LocalActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        self.inner.remove(&key)
            .map(|actor_ref| ActorRegistry::from_value::<NA>(actor_ref))
    }

    /// Lookup an actor.
    pub fn lookup<NA>(&mut self) -> Option<LocalActorRef<NA::Message>>
        where NA: NewActor + 'static,
    {
        let key = TypeId::of::<NA>();
        self.inner.get(&key)
            .map(|actor_ref| ActorRegistry::from_value_ref::<NA>(actor_ref))
    }

    /// Convert an local actor reference into a value that can be inserted into
    /// the hash map.
    #[inline]
    fn into_value<NA>(actor_ref: LocalActorRef<NA::Message>) -> *mut ()
        where NA: NewActor + 'static,
    {
        unsafe { mem::transmute(actor_ref) }
    }

    /// Convert a value coming from the hash map into an actor reference.
    #[inline]
    fn from_value<NA>(value: *mut ()) -> LocalActorRef<NA::Message>
        where NA: NewActor + 'static,
    {
        unsafe { mem::transmute(value) }
    }

    /// Convert a value coming from the hash map into an actor reference.
    #[inline]
    fn from_value_ref<NA>(value: &*mut ()) -> LocalActorRef<NA::Message>
        where NA: NewActor + 'static,
    {
        let actor_ref_ref: &LocalActorRef<NA::Message> = unsafe { mem::transmute(value) };
        actor_ref_ref.clone()
    }
}
