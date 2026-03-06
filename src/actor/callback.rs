#![allow(warnings)] // TODO.

use std::marker::PhantomData;

use crate::actor::{self, Actor, NewActor};

pub struct CallbackActor<C, S, M, RT> {
    /// Required parameters to implement `NewActor` for this type.
    _phantom: PhantomData<fn(actor::Context<M, RT>, C, S)>,
}

impl<C, S, M, RT, E> NewActor for CallbackActor<C, S, M, RT>
where
    C: FnMut(&mut RT, &mut S, M) -> Result<(), E>,
{
    type Message = M;
    type Argument = (S, C);
    type Actor = impl Actor<Error = E>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        (state, callback): Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(callback_actor(ctx, state, callback))
    }

    fn name() -> &'static str {
        actor::name::<C>()
    }
}

async fn callback_actor<M, RT, S, C, E>(
    mut ctx: actor::Context<M, RT>,
    mut state: S,
    mut callback: C,
) -> Result<(), E>
where
    C: FnMut(&mut RT, &mut S, M) -> Result<(), E>,
{
    while let Ok(msg) = ctx.receive_next().await {
        callback(ctx.runtime(), &mut state, msg)?;
    }
    Ok(())
}
