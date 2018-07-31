//! Module with time related utilities.

use std::task::{Context, Poll};
use std::future::Future;
use std::mem::PinMut;
use std::time::{Duration, Instant};

use mio_st::timer::Timer as MioTimer;
use mio_st::poll::PollOption;
use mio_st::event::Ready;

use crate::actor::ActorContext;

/// A future that represents a timer.
///
/// If this future returns `Poll::Ready(DeadlinePassed)` it means that the
/// deadline has passed. If it returns `Poll::Pending` it's not yet passed.
///
/// # Examples
///
/// Using the `select!` macro to add a timeout to receiving a message.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, pin, never_type)]
///
/// use std::time::Duration;
///
/// use actor::actor::{ActorContext, actor_factory};
/// use actor::timer::Timer;
/// use futures_util::select;
///
/// async fn print_actor(mut ctx: ActorContext<String>, item: ()) -> Result<(), !> {
///     loop {
///         // Create future timer, this will be ready once the timeout has
///         // passed.
///         let mut timeout = Timer::timeout(&mut ctx, Duration::from_millis(100));
///         // Create a future to receive a message.
///         let mut msg = ctx.receive();
///
///         // Now let them race!
///         // This is basically a match statement for futures, whichever
///         // future returns first will be the winner and we'll take that
///         // branch.
///         let msg = select! {
///             msg => msg,
///             timeout => {
///                 println!("Getting impatient!");
///                 continue;
///             },
///         };
///
///         println!("Got a message: {}", msg);
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Timer {
    deadline: Instant,
}

impl Timer {
    /// Create a new timeout.
    ///
    /// Same as calling `Timer::deadline(ctx, Instant::now() + timeout)`.
    pub fn timeout<M>(ctx: &mut ActorContext<M>, timeout: Duration) -> Timer {
        Timer::deadline(ctx, Instant::now() + timeout)
    }

    /// Create a new timeout with a specific deadline.
    pub fn deadline<M>(ctx: &mut ActorContext<M>, deadline: Instant) -> Timer {
        let id = ctx.pid().into();
        let system_ref = ctx.system_ref();
        let mut timer = MioTimer::deadline(deadline);
        system_ref.poller_register(&mut timer, id, Ready::TIMER,
            PollOption::Oneshot).unwrap();
        // It's safe to drop `timer` here.
        Timer {
            deadline,
        }
    }
}

/// Type returned when the deadline has passed.
///
/// See [`Timer`].
///
/// [`Timer`]: struct.Timer.html
#[derive(Copy, Clone, Debug)]
pub struct DeadlinePassed;

impl Future for Timer {
    type Output = DeadlinePassed;

    fn poll(self: PinMut<Self>, _ctx: &mut Context) -> Poll<Self::Output> {
        if self.deadline <= Instant::now() {
            Poll::Ready(DeadlinePassed)
        } else {
            Poll::Pending
        }
    }
}
