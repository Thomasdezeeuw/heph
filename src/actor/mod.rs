//! The module with the actor trait and related definitions.
//!
//! Actors come in two flavours:
//!
//! * Asynchronous actors, and
//! * Synchronous actors.
//!
//! The following sections describe each kind of actor.
//!
//! ## Asynchronous actors
//!
//! Asynchronous actors must implement the [`Actor`] trait, which defines how an
//! actor is run. The [`NewActor`] defines how an actor is created and is used
//! in staring, or spawning, new actors. The easiest way to implement these
//! traits is to use asynchronous functions, see the example below.
//!
//! Asynchronous are [`Future`] which means that they can share a single OS
//! thread, but may not block that thread as it can block countless other actors
//! (and futures). Furthermore the actors must be run by a `Future` runtime. One
//! is provided in the [Heph-rt] crate. If you want to use another `Future`
//! runtime take a look at the [`ActorFuture`] type.
//!
//! The example below shows how we can use an asynchronous function to implement
//! the `NewActor` and `Actor` traits.
//!
//! ```
//! use heph::actor::{self, NewActor};
//!
//! async fn actor(ctx: actor::Context<()>) {
//! #   drop(ctx); // Use `ctx` to silence dead code warnings.
//!     println!("Actor is running!");
//! }
//!
//! // Unfortunately `actor` doesn't yet implement `NewActor`, it first needs
//! // to be cast into a function pointer, which does implement `NewActor`.
//! use_actor(actor as fn(_) -> _);
//!
//! fn use_actor<NA>(new_actor: NA) where NA: NewActor {
//!     // Do stuff with the actor ...
//! #   drop(new_actor);
//! }
//! ```
//!
//! [Heph-rt]: https://crates.io/crates/heph-rt
//!
//! ## Synchronous actors
//!
//! Asynchronous actors are not allowed to block the thread they run on, as that
//! would block all other actors running on that thread as well. However
//! sometimes blocking operations is exactly what we need to do, for that
//! purpose Heph has synchronous actors.
//!
//! Synchronous actors must implement the [`SyncActor`] trait, which defines how
//! a synchronous actor is run. There is no `NewActor` equivalent as `SyncActor`
//! defines both the creation and running of the actor.
//!
//! Synchronous actors run in their own thread and can use blocking operations,
//! such as blocking I/O or heavy computation. Instead of an [`actor::Context`]
//! they use a [`SyncContext`], which provides a similar API to
//! `actor::Context`, but uses blocking operations. As each synchronous actor
//! requires their own thread to run on, they are more expansive to run than
//! asynchronous actors (by an order of a magnitude).
//!
//! Synchronous actors can be spawned using [`spawn_sync_actor`]. Note that the
//! Heph-rt crate provides function to spawn synchronous actors that it manages
//! for you.
//!
//! The example below shows how to run a synchronous actor.
//!
//! ```
//! use heph::actor::{SyncContext, spawn_sync_actor};
//! use heph::supervisor::NoSupervisor;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Same as we saw for the asynchronous actors, we have to cast the function
//! // to a function pointer for it to implement `SyncActor`.
//! let actor = actor as fn(_) -> _;
//! let (thread_handle, actor_ref) = spawn_sync_actor(NoSupervisor, actor, (), ())?;
//!
//! // We can send it a message like we do with asynchronous actors.
//! actor_ref.try_send("Hello world!")?;
//!
//! // Wait for the actor to complete.
//! thread_handle.join().unwrap();
//! # Ok(())
//! # }
//!
//! fn actor(mut ctx: SyncContext<String>) {
//!     if let Ok(msg) = ctx.receive_next() {
//!         println!("Got a message: {msg}");
//!     } else {
//!         eprintln!("Receive no messages");
//!     }
//! }
//! ```
//!
//! [`actor::Context`]: Context

use std::any::type_name;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{self, Poll};

mod context;
mod future;
mod sync;
#[cfg(test)]
mod tests;

#[doc(inline)]
pub use context::{Context, NoMessages, ReceiveMessage, RecvError};
#[doc(inline)]
pub use future::ActorFuture;
#[doc(inline)]
pub use sync::{spawn_sync_actor, SyncActor, SyncContext};

#[doc(hidden)] // Not part of the stable API.
pub use sync::SyncWaker;

/// Creating asynchronous actors.
///
/// The trait that defines how to create a new [`Actor`]. The easiest way to
/// implement this by using an asynchronous function, see the [actor module]
/// documentation.
///
/// [actor module]: crate::actor
pub trait NewActor {
    /// The type of messages the actor can receive.
    ///
    /// Using an enum allows an actor to handle multiple types of messages.
    ///
    /// # Examples
    ///
    /// Here is an example of using an enum as message type.
    ///
    /// ```
    /// # #![feature(never_type)]
    /// # use heph::supervisor::NoSupervisor;
    /// # use heph::{actor, from_message};
    /// # use heph_rt::spawn::ActorOptions;
    /// # use heph_rt::{self as rt, Runtime, ThreadLocal};
    /// #
    /// # fn main() -> Result<(), rt::Error> {
    /// #     // Create and run the runtime.
    /// #     let mut runtime = Runtime::new()?;
    /// #     runtime.run_on_workers(|mut runtime_ref| -> Result<(), !> {
    /// #         // Spawn the actor.
    /// #         let new_actor = actor as fn(_) -> _;
    /// #         let actor_ref = runtime_ref.spawn_local(NoSupervisor, new_actor, (), ActorOptions::default());
    /// #
    /// #         // Now we can use the reference to send the actor a message. We
    /// #         // don't have to use `Message` type we can just use `String`,
    /// #         // because `Message` implements `From<String>`.
    /// #         actor_ref.try_send("Hello world".to_owned()).unwrap();
    /// #         Ok(())
    /// #     })?;
    /// #     runtime.start()
    /// # }
    /// #
    /// /// The message type for the actor.
    /// #[derive(Debug)]
    /// # #[derive(Eq, PartialEq)]
    /// enum Message {
    ///     String(String),
    /// #   #[allow(dead_code)]
    ///     Number(usize),
    /// }
    ///
    /// // Implementing `From` for the message allows us to just pass a
    /// // `String`, rather than a `Message::String`.
    /// from_message!(Message::String(String));
    /// from_message!(Message::Number(usize));
    ///
    /// /// Our actor implementation that prints all messages it receives.
    /// async fn actor(mut ctx: actor::Context<Message, ThreadLocal>) {
    ///     if let Ok(msg) = ctx.receive_next().await {
    /// #       assert_eq!(msg, Message::String("Hello world".to_owned()));
    ///         println!("received message: {msg:?}");
    ///     }
    /// }
    /// ```
    type Message;

    /// The argument(s) passed to the actor.
    ///
    /// The arguments passed to the actor are much like arguments passed to a
    /// regular function. If more than one argument is needed the arguments can
    /// be in the form of a tuple, e.g. `(usize, String)`.
    ///
    /// An empty tuple can be used for actors that don't accept any arguments
    /// (except for the `actor::Context`, see [`new`] below).
    ///
    /// When using asynchronous functions arguments are passed regularly, i.e.
    /// not in the form of a tuple, see there [implementations] below. However
    /// they do have be passed as a tuple to [`ActorFuture::new`].
    ///
    /// [`new`]: NewActor::new
    /// [implementations]: #foreign-impls
    type Argument;

    /// The type of the actor.
    ///
    /// See [`Actor`](Actor) for more.
    type Actor: Actor;

    /// The type of error.
    ///
    /// Note that if creating an actor is always successful the never type (`!`)
    /// can be used. Asynchronous functions for example use the never type as
    /// error.
    type Error;

    /// The kind of runtime access needed by the actor.
    ///
    /// The runtime is accessible via the actor's context. This can be accessed
    /// by the actor via the [`actor::Context`] (the `RT` generic parameter).
    ///
    /// [`actor::Context`]: crate::actor::Context
    type RuntimeAccess;

    /// Create a new [`Actor`](Actor).
    #[allow(clippy::wrong_self_convention)]
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error>;

    /// Wrap the `NewActor` to change the arguments its accepts.
    ///
    /// This can be used when additional arguments are needed to be passed to an
    /// actor, where another function requires a certain argument list.
    ///
    /// # Examples
    ///
    /// Using TCP server (from the Heph-rt crate) requires a `NewActor` that
    /// accepts `TcpStream` as arguments, but we need to pass the actor
    /// additional arguments.
    ///
    /// ```
    /// # #![feature(never_type)]
    /// use std::io;
    /// use heph::actor::{self, NewActor};
    /// # use heph::messages::Terminate;
    /// use heph_rt::net::{tcp, TcpStream};
    /// # use heph_rt::net::tcp::server;
    /// use heph_rt::spawn::ActorOptions;
    /// use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
    /// # use heph::supervisor::{Supervisor, SupervisorStrategy};
    /// # use log::error;
    ///
    /// fn main() -> Result<(), rt::Error> {
    ///     // Create and run runtime.
    ///     let mut runtime = Runtime::new()?;
    ///     runtime.run_on_workers(setup)?;
    ///     runtime.start()
    /// }
    ///
    /// /// In this setup function we'll spawn the TCP server actor.
    /// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
    ///     // Prepare for humans' expansion to Mars.
    ///     let greet_mars = true;
    ///
    ///     // We convert our actor that accepts three arguments into an actor
    ///     // that accept two arguments and gets `greet_mars` passed to it as
    ///     // third argument.
    ///     let new_actor = (conn_actor as fn(_, _, _) -> _)
    ///         .map_arg(move |stream| (stream, greet_mars));
    ///
    ///     // For more information about the remainder of this example see
    ///     // the `net::tcp::server` module in the heph-rt crate.
    ///     let address = "127.0.0.1:7890".parse().unwrap();
    ///     let server = tcp::server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
    ///     # let actor_ref =
    ///     runtime_ref.spawn_local(ServerSupervisor, server, (), ActorOptions::default());
    ///     # actor_ref.try_send(Terminate).unwrap();
    ///     Ok(())
    /// }
    ///
    /// # #[derive(Copy, Clone, Debug)]
    /// # struct ServerSupervisor;
    /// #
    /// # impl<S, NA> Supervisor<server::Setup<S, NA>> for ServerSupervisor
    /// # where
    /// #     S: Supervisor<NA> + Clone + 'static,
    /// #     NA: NewActor<Argument = TcpStream, Error = !, RuntimeAccess = ThreadLocal> + Clone + 'static,
    /// # {
    /// #     fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
    /// #         use server::Error::*;
    /// #         match err {
    /// #             Accept(err) => {
    /// #                 error!("error accepting new connection: {err}");
    /// #                 SupervisorStrategy::Restart(())
    /// #             }
    /// #             NewActor(_) => unreachable!(),
    /// #         }
    /// #     }
    /// #
    /// #     fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<()> {
    /// #         err
    /// #     }
    /// #
    /// #     fn second_restart_error(&mut self, err: !) {
    /// #         err
    /// #     }
    /// # }
    /// #
    /// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
    /// #   error!("error handling connection: {err}");
    /// #   SupervisorStrategy::Stop
    /// # }
    /// #
    /// // Actor that handles a connection.
    /// async fn conn_actor(
    ///     ctx: actor::Context<!, ThreadLocal>,
    ///     mut stream: TcpStream,
    ///     greet_mars: bool
    /// ) -> io::Result<()> {
    /// #   drop(ctx); // Silence dead code warnings.
    ///     if greet_mars {
    ///         // In case this example ever reaches Mars.
    ///         stream.send_all("Hello Mars").await?;
    ///     } else {
    ///         stream.send_all("Hello World").await?;
    ///     }
    ///     Ok(())
    /// }
    /// ```
    fn map_arg<F, Arg>(self, f: F) -> ArgMap<Self, F, Arg>
    where
        Self: Sized,
        F: FnMut(Arg) -> Self::Argument,
    {
        ArgMap {
            new_actor: self,
            map: f,
            _phantom: PhantomData,
        }
    }

    /// Returns the name of the actor.
    ///
    /// The default implementation creates the name based on the name of type of
    /// the actor.
    ///
    /// # Notes
    ///
    /// This uses [`type_name`] under the hood which does not have a stable
    /// output. Like the `type_name` function the default implementation is
    /// provided on a best effort basis.
    fn name() -> &'static str {
        name::<Self::Actor>()
    }
}

/// See [`NewActor::map_arg`].
#[derive(Debug)]
pub struct ArgMap<NA, F, Arg> {
    new_actor: NA,
    map: F,
    _phantom: PhantomData<Arg>,
}

impl<NA, F, Arg> Clone for ArgMap<NA, F, Arg>
where
    NA: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        ArgMap {
            new_actor: self.new_actor.clone(),
            map: self.map.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<NA, F, Arg> NewActor for ArgMap<NA, F, Arg>
where
    NA: NewActor,
    F: FnMut(Arg) -> NA::Argument,
{
    type Message = NA::Message;
    type Argument = Arg;
    type Actor = NA::Actor;
    type Error = NA::Error;
    type RuntimeAccess = NA::RuntimeAccess;

    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let arg = (self.map)(arg);
        self.new_actor.new(ctx, arg)
    }

    fn name() -> &'static str {
        NA::name()
    }
}

/// Macro to implement the [`NewActor`] trait on function pointers.
macro_rules! impl_new_actor {
    (
        $( ( $( $arg_name: ident : $arg: ident ),* ) ),*
        $(,)?
    ) => {
        $(
            impl<M, RT, $( $arg, )* A> NewActor for fn(ctx: Context<M, RT>, $( $arg_name: $arg ),*) -> A
            where
                A: Actor,
            {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Actor = A;
                type Error = !;
                type RuntimeAccess = RT;

                #[allow(non_snake_case)]
                fn new(
                    &mut self,
                    ctx: Context<Self::Message, Self::RuntimeAccess>,
                    arg: Self::Argument,
                ) -> Result<Self::Actor, Self::Error> {
                    let ($( $arg ),*) = arg;
                    Ok((self)(ctx, $( $arg ),*))
                }
            }
        )*
    };
}

// `NewActor` with no arguments.
impl_new_actor!(());

impl<M, RT, Arg, A> NewActor for fn(ctx: Context<M, RT>, arg: Arg) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = Arg;
    type Actor = A;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg))
    }
}

impl_new_actor!(
    // NOTE: we don't want a single argument into tuple form so we implement
    // that manually above.
    (arg1: Arg1, arg2: Arg2),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4, arg5: Arg5),
);

/// Asynchronous actor.
///
/// Effectively an `Actor` is a [`Future`] which returns a result. All `Future`s
/// where the [`Output`] type is `Result<(), Error>` or `()` implement the
/// `Actor` trait.
///
/// The easiest way to implement this by using an async function, see the
/// [actor module] documentation.
///
/// [`Output`]: Future::Output
/// [actor module]: crate::actor
///
/// # Panics
///
/// Because this is basically a [`Future`] it also shares it's characteristics,
/// including it's unsafety. Please read the [`Future`] documentation when
/// implementing or using this trait manually.
#[must_use = "asynchronous actors do nothing unless you poll them"]
pub trait Actor {
    /// An error the actor can return to its [supervisor]. This error will be
    /// considered terminal for this actor and should **not** be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// is up to the actor.
    ///
    /// [supervisor]: crate::supervisor::Supervisor
    type Error;

    /// Try to poll this actor.
    ///
    /// This is basically the same as calling [`Future::poll`].
    ///
    /// # Panics
    ///
    /// Just like with [`Future`]s polling after it returned [`Poll::Ready`] may
    /// cause undefined behaviour, including but not limited to panicking.
    fn try_poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>)
        -> Poll<Result<(), Self::Error>>;
}

/// Supported are [`Future`]s with `Result<(), E>` or `()` [`Output`].
///
/// [`Output`]: Future::Output
impl<Fut, O, E> Actor for Fut
where
    Fut: Future<Output = O>,
    O: private::ActorResult<Error = E>,
{
    type Error = E;

    fn try_poll(
        self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.poll(ctx).map(private::ActorResult::into)
    }
}

mod private {
    /// Trait to support [`Actor`] for `Result<(), E>` and `()`.
    ///
    /// [`Actor`]: crate::actor::Actor
    pub trait ActorResult {
        /// See [`Actor::Error`].
        ///
        /// [`Actor::Error`]: crate::actor::Actor::Error
        type Error;

        /// Convert the return type in an `Result<(), Self::Error>`.
        fn into(self) -> Result<(), Self::Error>;
    }

    impl<E> ActorResult for Result<(), E> {
        type Error = E;

        fn into(self) -> Result<(), E> {
            self
        }
    }

    impl ActorResult for () {
        type Error = !;

        fn into(self) -> Result<(), !> {
            Ok(())
        }
    }
}

/// Returns the name for actors of type `A`.
///
/// This is the default implementation of [`NewActor::name`].
#[doc(hidden)] // Not part of the stable API.
pub fn name<A>() -> &'static str {
    format_name(type_name::<A>())
}

// NOTE: split for easier testing.
fn format_name(full_name: &'static str) -> &'static str {
    const GEN_FUTURE: &str = "GenFuture<";
    const GENERIC_START: &str = "<";
    const GENERIC_END: &str = ">";
    const CLOSURE: &str = "{{closure}}";

    let mut name = full_name;

    if name.starts_with("fn(") {
        // If this function is called with a type converted into a function
        // pointer (using `fn(_) -> _`) there is little we can do to improve the
        // readability, so we just return the full name.
        return name;
    }

    // Async functions are wrapped in the `GenFuture` type; remove that, e.g.
    // from
    // `core::future::from_generator::GenFuture<1_hello_world::greeter_actor::{{closure}}>`
    // to `1_hello_world::greeter_actor::{{closure}}`.
    match (name.find(GEN_FUTURE), name.find(GENERIC_START)) {
        (Some(start_index), Some(i)) if start_index < i => {
            // Outer type is `GenFuture`, remove that.
            name = &name[start_index + GEN_FUTURE.len()..name.len() - GENERIC_END.len()];

            // Async functions often end with `::{{closure}}`; also remove that,
            // e.g. from `1_hello_world::greeter_actor::{{closure}}` to
            // `1_hello_world::greeter_actor`.
            if let Some(start_index) = name.rfind("::") {
                let last_part = &name[start_index + 2..];
                if last_part == CLOSURE {
                    name = &name[..start_index];
                }
            }

            // Remove generic parameters, e.g.
            // `deadline_actor<heph::actor::context::ThreadLocal>` to
            // `deadline_actor`.
            if name.ends_with('>') {
                if let Some(start_index) = name.find('<') {
                    name = &name[..start_index];
                }
            }

            // Function named `actor` in a module named `actor`. We'll drop the
            // function name and keep the `actor` part of the module, e.g.
            // `storage::actor::actor` to `storage::actor`.
            if name.ends_with("actor::actor") {
                // Remove `::actor`.
                name = &name[..name.len() - 7];
            }

            // Either take the last part of the name's path, e.g. from
            // `1_hello_world::greeter_actor` to `greeter_actor`.
            // Or keep the module name in case the actor's name would be `actor`,
            // e.g. from `1_hello_world::some::nested::module::greeter::actor` to
            // `greeter::actor`.
            if let Some(start_index) = name.rfind("::") {
                let actor_name = &name[start_index + 2..];
                if actor_name == "actor" {
                    // If the actor's name is `actor` will keep the last module
                    // name as part of the name.
                    if let Some(module_index) = name[..start_index].rfind("::") {
                        name = &name[module_index + 2..];
                    } // Else only a single module in path.
                } else {
                    name = actor_name;
                }
            }
        }
        _ => {
            // Otherwise we trait it like a normal type and remove the generic
            // parameters from it, e.g. from
            // `heph::net::tcp::server::TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context_priv::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context::ThreadLocal>`
            // to `heph::net::tcp::server::TcpServer`.
            if let Some(start_index) = name.find(GENERIC_START) {
                name = &name[..start_index];

                if let Some(start_index) = name.rfind("::") {
                    // Next we remove the module path, e.g. from
                    // `heph::net::tcp::server::TcpServer` to `TcpServer`.
                    name = &name[start_index + 2..];
                }
            }
        }
    }

    name
}
