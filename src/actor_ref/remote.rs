//! Module containing the `RemoteActorRef`.

use std::fmt;
use std::marker::PhantomData;

use crate::error::SendError;

/// This is currently not implemented.
pub struct RemoteActorRef<M>{
    _phantom: PhantomData<M>,
}

impl<M> RemoteActorRef<M> {
    /// This is currently not implemented.
    pub fn send<Msg>(&mut self, _msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        unimplemented!("RemoteActorRef.send");
    }
}

impl<M> fmt::Debug for RemoteActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("RemoteActorRef")
            .finish()
    }
}
