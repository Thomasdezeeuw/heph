//! Module containing the `RemoteActorRef`.

use std::fmt;
use std::marker::PhantomData;
use std::ops::ShlAssign;

/// This is currently not implemented.
pub struct RemoteActorRef<M> {
    _phantom: PhantomData<M>,
}

impl<M> RemoteActorRef<M> {
    /// This is currently not implemented.
    pub fn send<Msg>(&mut self, _msg: Msg)
        where Msg: Into<M>,
    {
        unimplemented!("RemoteActorRef.send");
    }
}

impl<M, Msg> ShlAssign<Msg> for RemoteActorRef<M>
    where Msg: Into<M>
{
    fn shl_assign(&mut self, msg: Msg) {
        self.send(msg);
    }
}

impl<M> fmt::Debug for RemoteActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("RemoteActorRef")
            .finish()
    }
}
