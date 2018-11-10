//! The module with the supervisor, and related, types.
//!
//! # Supervisor
//!
//! A supervisor supervises an actor and handles its errors. A supervisor
//! generally does two things; logging and deciding whether to stop or restart
//! the actor. Its advised to keep a supervisor small and simple.
//!
//! When encountering an error it usually means someone has to be notified (to
//! fix it), something often done via logging.
//!
//! Next the supervisor needs to decide if the actor needs to be [stopped] or
//! [restarted]. If the supervisor decides to restart the actor it needs to
//! provide the argument to create a new actor (used in calling the
//! [`NewActor.new`] method).
//!
//! The restarted actor will have the same message inbox as the old (stopped)
//! actor. Note however that if an actor retrieved a message from its inbox, and
//! returned an error when processing it, the new (restarted) actor won't
//! retrieve that message again (messages aren't cloned after all).
//!
//! # Restarting or stopping?
//!
//! Sometimes just restarting an actor is the easiest way to deal with errors.
//! Starting the actor from a clean slate will often allow it to continue
//! processing. However this is not possible in all cases, for example when a
//! new argument can't be provided (think actors started by a [`TcpListener`]).
//! In those cases the supervisor should still log the error encountered.
//!
//! # Examples
//!
//! TODO: add example.
//!
//! [stopped]: enum.SupervisorStrategy.html#variant.Stop
//! [restarted]: enum.SupervisorStrategy.html#variant.Restart
//! [`NewActor.new`]: ../actor/trait.NewActor.html#tymethod.new
//! [`TcpListener`]: ../net/struct.TcpListener.html

/// The supervisor trait.
///
/// For more information about supervisors see the [module documentation], here
/// only the design of the trait is discussed.
///
/// The trait is designed to be generic to the error (`E`) and argument used in
/// (re)starting the actor (`Arg`). This means that the same type can implement
/// supervisor for a number of different actors. But a word of caution,
/// supervisors should be small and simple, which means that most times having a
/// different supervisor for each actor is a good thing.
///
/// Supervisor is implemented for any function that takes an error `E` and
/// returns `SupervisorStrategy<Arg>` automatically.
///
/// [module documentation]: index.html
pub trait Supervisor<E, Arg> {
    /// Decide what happens to the actor that returned `error`.
    fn decide(&mut self, error: E) -> SupervisorStrategy<Arg>;
}

/// The strategy to use when handling an error from an actor.
///
/// See the [module documentation] for deciding on whether to restart an or not.
///
/// [module documentation]: index.html#restarting-or-stopping
#[derive(Debug)]
pub enum SupervisorStrategy<Arg> {
    /// Restart the actor with the provided argument `Arg`.
    Restart(Arg),
    /// Stop the actor.
    Stop,
}

impl<F, E, Arg> Supervisor<E, Arg> for F
    where F: FnMut(E) -> SupervisorStrategy<Arg>,
{
    fn decide(&mut self, error: E) -> SupervisorStrategy<Arg> {
        (self)(error)
    }
}
