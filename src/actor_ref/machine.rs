//! Module containing the `MachineLocalActorRef`.

use std::fmt;
use std::marker::PhantomData;

use crate::error::SendError;

/// This is currently not implemented.
pub struct MachineLocalActorRef<M> {
    _phantom: PhantomData<M>,
}

impl<M> MachineLocalActorRef<M> {
    /// This is currently not implemented.
    pub fn send<Msg>(&mut self, _msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        unimplemented!("MachineLocalActorRef.send");
    }
}

impl<M> fmt::Debug for MachineLocalActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MachineLocalActorRef")
            .finish()
    }
}
