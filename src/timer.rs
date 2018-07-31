//! Module with time related utilities.
//!
//! This module provides two types [`Timer`] and [`Deadline`], both of which
//! implement `Future`. The difference is that `Timer` is a stand-alone future
//! that returns [`DeadlinePassed`] and `Deadline` wraps another `Future` and
//! checks the deadline each time it's polled, it returns `Result<T,
//! DeadlinePassed>`.
//!
//! [`Timer`]: struct.Timer.html
//! [`Deadline`]: struct.Deadline.html
//! [`DeadlinePassed`]: struct.DeadlinePassed.html

use std::task::{Context, Poll};
use std::future::Future;
use std::mem::PinMut;
use std::time::{Duration, Instant};

use mio_st::timer::Timer as MioTimer;
use mio_st::poll::PollOption;
use mio_st::event::Ready;

use crate::actor::ActorContext;

/// Type returned when the deadline has passed.
///
/// See [`Timer`].
///
/// [`Timer`]: struct.Timer.html
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DeadlinePassed;

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
    /// Create a new timer.
    ///
    /// Same as calling `Timer::deadline(ctx, Instant::now() + timeout)`.
    pub fn timeout<M>(ctx: &mut ActorContext<M>, timeout: Duration) -> Timer {
        Timer::deadline(ctx, Instant::now() + timeout)
    }

    /// Create a new timer with a specific deadline.
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

/// A future that represents a deadline.
///
/// When this future is polled it first checks if the deadline has passed, if so
/// it returns `Poll::Ready(Err(DeadlinePassed))`. Otherwise this will call the
/// provided future.
///
/// # Examples
///
/// Using the `select!` macro to add a timeout to receiving a message.
///
/// ```
/// #![feature(async_await, await_macro, futures_api, pin, never_type)]
/// # #![feature(arbitrary_self_types)]
///
/// # use std::future::Future;
/// # use std::mem::PinMut;
/// # use std::task::{Context, Poll};
/// use std::thread::sleep;
/// use std::time::Duration;
///
/// use actor::actor::{ActorContext, actor_factory};
/// use actor::timer::{DeadlinePassed, Deadline};
/// use futures_util::select;
///
/// # struct OtherFuture;
/// #
/// # impl Future for OtherFuture {
/// #     type Output = ();
/// #     fn poll(self: PinMut<Self>, ctx: &mut Context) -> Poll<Self::Output> {
/// #         unimplemented!();
/// #     }
/// # }
/// #
/// async fn print_actor(mut ctx: ActorContext<String>, item: ()) -> Result<(), !> {
///     // OtherFuture is a type the implements `Future`.
///     let future = OtherFuture;
///     // Create our deadline.
///     let deadline_future = Deadline::timeout(&mut ctx, Duration::from_millis(100), future);
///
///     // Sleep a bit to fake work.
///     sleep(Duration::from_millis(100));
///
///     let result = await!(deadline_future);
///     assert_eq!(result, Err(DeadlinePassed));
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct Deadline<Fut> {
    deadline: Instant,
    fut: Fut,
}

impl<Fut> Deadline<Fut> {
    /// Create a new deadline.
    ///
    /// Same as calling `Deadline::deadline(ctx, Instant::now() + timeout, fut)`.
    pub fn timeout<M>(ctx: &mut ActorContext<M>, timeout: Duration, fut: Fut) -> Deadline<Fut> {
        Deadline::deadline(ctx, Instant::now() + timeout, fut)
    }

    /// Create a new deadline with a specific deadline.
    pub fn deadline<M>(ctx: &mut ActorContext<M>, deadline: Instant, fut: Fut) -> Deadline<Fut> {
        let id = ctx.pid().into();
        let system_ref = ctx.system_ref();
        let mut timer = MioTimer::deadline(deadline);
        system_ref.poller_register(&mut timer, id, Ready::TIMER,
            PollOption::Oneshot).unwrap();
        // It's safe to drop `timer` here.
        Deadline {
            deadline,
            fut,
        }
    }
}

impl<Fut> Future for Deadline<Fut>
    where Fut: Future,
{
    type Output = Result<Fut::Output, DeadlinePassed>;

    fn poll(self: PinMut<Self>, ctx: &mut Context) -> Poll<Self::Output> {
        if self.deadline <= Instant::now() {
            Poll::Ready(Err(DeadlinePassed))
        } else {
            let this = unsafe { PinMut::get_mut_unchecked(self) };
            let future = unsafe { PinMut::new_unchecked(&mut this.fut) };
            future.poll(ctx).map(|output| Ok(output))
        }
    }
}
