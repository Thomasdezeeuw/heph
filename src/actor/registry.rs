use std::any::TypeId;
use std::collections::hash_map::{Entry, HashMap};
use std::lazy::SyncLazy;
use std::sync::RwLock;

use crate::ActorRef;

static REGISTRY: SyncLazy<RwLock<HashMap<Key, UntypedActorRef>>> =
    SyncLazy::new(|| RwLock::new(HashMap::new()));

/// Lookup an actor with `name` and message type `M`.
///
/// # Notes
///
/// If the message type `M` is incorrect this will return `None`.
pub fn lookup<M: 'static>(name: &'static str) -> Option<ActorRef<M>> {
    let key = Key::new::<M>(name);
    assert!(key.same_type::<M>());
    todo!()
}

/// Register an actor with `name`.
pub fn register<M: 'static>(
    name: &'static str,
    actor_ref: ActorRef<M>,
) -> Result<(), RegisterError> {
    let key = Key::new::<M>(name);
    let untyped_ref = UntypedActorRef::from_ref(actor_ref);
    let mut registry = REGISTRY.write().unwrap();
    match registry.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(untyped_ref);
            Ok(())
        }
        _ => {
            drop(untyped_ref.into_ref::<M>());
            Err(RegisterError {})
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
struct Key {
    type_id: TypeId,
    name: &'static str,
}

impl Key {
    /// Create a new `Key` using `name` and message type `M`.
    const fn new<M: 'static>(name: &'static str) -> Key {
        Key {
            type_id: TypeId::of::<M>(),
            name,
        }
    }

    /// Return `true` if the key is for type `M`, false otherwise.
    fn same_type<M: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<M>()
    }
}

pub struct UntypedActorRef {
    // TODO.
// FIXME: leaks memory.
}

impl UntypedActorRef {
    fn from_ref<M>(actor_ref: ActorRef<M>) -> UntypedActorRef {
        todo!()
    }

    fn into_ref<M>(self) -> ActorRef<M> {
        todo!()
    }

    fn clone_into_ref<M>(&self) -> ActorRef<M> {
        todo!()
    }
}

pub struct RegisterError {
    // TODO.
}
