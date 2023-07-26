//! Time related utilities.
//!
//! This module provides three types.
//!
//! - [`Timer`] is a stand-alone [`Future`] that returns [`DeadlinePassed`] once
//!   the deadline has passed.
//! - [`Deadline`] wraps another `Future` and checks the deadline each time it's
//!   polled.
//! - [`Interval`] implements [`AsyncIterator`] which yields an item after the
//!   deadline has passed each interval.

use std::async_iter::AsyncIterator;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use crate::access::Access;
use crate::timers::TimerToken;
use crate::wakers::create_no_ring_waker;

/// Type returned when the deadline has passed.
///
/// Can be converted into [`io::ErrorKind::TimedOut`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DeadlinePassed;

impl From<DeadlinePassed> for io::Error {
    fn from(_: DeadlinePassed) -> io::Error {
        io::ErrorKind::TimedOut.into()
    }
}

impl From<DeadlinePassed> for io::ErrorKind {
    fn from(_: DeadlinePassed) -> io::ErrorKind {
        io::ErrorKind::TimedOut
    }
}

/// A [`Future`] that represents a timer.
///
/// If this future returns [`Poll::Ready`]`(`[`DeadlinePassed`]`)` it means that
/// the deadline has passed. If it returns [`Poll::Pending`] the deadline has
/// not yet passed.
///
/// # Examples
///
/// ```
/// # #![feature(never_type)]
/// #
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::actor::actor_fn;
/// # use heph::supervisor::NoSupervisor;
/// # use heph_rt::spawn::ActorOptions;
/// # use heph_rt::{self as rt, Runtime, RuntimeRef};
/// use heph_rt::ThreadLocal;
/// use heph_rt::timer::Timer;
///
/// # fn main() -> Result<(), rt::Error> {
/// #     let mut runtime = Runtime::new()?;
/// #     runtime.run_on_workers(setup)?;
/// #     runtime.start()
/// # }
/// #
/// #
/// # fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
/// #   runtime_ref.spawn_local(NoSupervisor, actor_fn(actor), (), ActorOptions::default());
/// #   Ok(())
/// # }
/// #
/// async fn actor(ctx: actor::Context<!, ThreadLocal>) {
/// #   let start = Instant::now();
///     // Create a timer, this will be ready once the timeout has passed.
///     let timeout = Timer::after(ctx.runtime_ref().clone(), Duration::from_millis(200));
/// #   assert!(timeout.deadline() >= start + Duration::from_millis(200));
///
///     // Wait for the timer to pass.
///     timeout.await;
/// #   assert!(Instant::now() >= start + Duration::from_millis(200));
///     println!("200 milliseconds have passed!");
/// }
/// ```
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Timer<RT: Access> {
    deadline: Instant,
    rt: RT,
    /// If `Some` it means we've added a timer that hasn't expired yet.
    timer_pending: Option<TimerToken>,
}

impl<RT: Access> Timer<RT> {
    /// Create a new `Timer`.
    pub const fn at(rt: RT, deadline: Instant) -> Timer<RT> {
        Timer {
            deadline,
            rt,
            timer_pending: None,
        }
    }

    /// Create a new timer, based on a timeout.
    ///
    /// Same as calling `Timer::at(rt, Instant::now() + timeout)`.
    pub fn after(rt: RT, timeout: Duration) -> Timer<RT> {
        Timer::at(rt, Instant::now() + timeout)
    }

    /// Returns the deadline set for this `Timer`.
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns `true` if the deadline has passed.
    pub fn has_passed(&self) -> bool {
        self.deadline <= Instant::now()
    }

    /// Wrap a future creating a new `Deadline`.
    pub const fn wrap<Fut>(self, future: Fut) -> Deadline<Fut, RT> {
        Deadline {
            timer: self,
            future,
        }
    }
}

impl<RT: Access> Future for Timer<RT> {
    type Output = DeadlinePassed;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.has_passed() {
            self.timer_pending = None;
            return Poll::Ready(DeadlinePassed);
        } else if self.timer_pending.is_none() {
            let deadline = self.deadline;
            let waker = create_no_ring_waker(ctx).unwrap_or_else(|| ctx.waker().clone());
            self.timer_pending = Some(self.rt.add_timer(deadline, waker));
        }
        Poll::Pending
    }
}

impl<RT: Access> Unpin for Timer<RT> {}

impl<RT: Access> Drop for Timer<RT> {
    fn drop(&mut self) {
        if let Some(token) = self.timer_pending {
            self.rt.remove_timer(self.deadline, token);
        }
    }
}

/// A [`Future`] that wraps another future setting a deadline for it.
///
/// When this future is polled it first checks if the underlying future `Fut`
/// can make progress. If it returns pending it will check if the deadline has
/// expired.
///
/// # Notes
///
/// This type can also be created using [`Timer::wrap`], this is useful when
/// dealing with lifetime issue, e.g. when calling
/// [`actor::Context::receive_next`] and wrapping that in a `Deadline`.
///
/// [`actor::Context::receive_next`]: heph::actor::Context::receive_next
///
/// # Examples
///
/// Setting a timeout for a future.
///
/// ```
/// use std::io;
/// # use std::future::Future;
/// # use std::pin::Pin;
/// # use std::task::{self, Poll};
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::actor::actor_fn;
/// # use heph::supervisor::NoSupervisor;
/// use heph_rt::ThreadSafe;
/// # use heph_rt::spawn::ActorOptions;
/// # use heph_rt::{self as rt, Runtime};
/// use heph_rt::timer::Deadline;
///
/// # fn main() -> Result<(), rt::Error> {
/// #     let actor = actor_fn(actor);
/// #     let options = ActorOptions::default();
/// #     let mut rt = Runtime::new()?;
/// #     let _ = rt.spawn(NoSupervisor, actor, (), options);
/// #     rt.start()
/// # }
/// #
/// # struct IoFuture;
/// #
/// # impl Future for IoFuture {
/// #     type Output = io::Result<()>;
/// #     fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
/// #         Poll::Pending
/// #     }
/// # }
/// #
/// async fn actor(ctx: actor::Context<String, ThreadSafe>) {
///     // `OtherFuture` is a type that implements `Future`.
///     let future = IoFuture;
///     // Create our deadline.
/// #   let start = Instant::now();
///     let deadline_future = Deadline::after(ctx.runtime_ref().clone(), Duration::from_millis(100), future);
/// #   assert!(deadline_future.deadline() >= start + Duration::from_millis(100));
///
///     // Now we await the results.
///     let result = deadline_future.await;
/// #   assert!(Instant::now() >= start + Duration::from_millis(100));
///     // However the other future is rather slow, so the timeout will pass.
///     assert!(result.is_err());
///     assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
/// }
/// ```
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Deadline<Fut, RT: Access> {
    timer: Timer<RT>,
    future: Fut,
}

impl<Fut, RT: Access> Deadline<Fut, RT> {
    /// Create a new `Deadline`.
    pub const fn at(rt: RT, deadline: Instant, future: Fut) -> Deadline<Fut, RT> {
        Deadline {
            timer: Timer::at(rt, deadline),
            future,
        }
    }

    /// Create a new deadline based on a timeout.
    ///
    /// Same as calling `Deadline::at(rt, Instant::now() + timeout, future)`.
    pub fn after(rt: RT, timeout: Duration, future: Fut) -> Deadline<Fut, RT> {
        Deadline::at(rt, Instant::now() + timeout, future)
    }

    /// Returns the deadline set.
    pub const fn deadline(&self) -> Instant {
        self.timer.deadline
    }

    /// Returns `true` if the deadline has passed.
    pub fn has_passed(&self) -> bool {
        self.timer.deadline <= Instant::now()
    }

    /// Returns a reference to the wrapped future.
    pub const fn get_ref(&self) -> &Fut {
        &self.future
    }

    /// Returns a mutable reference to the wrapped future.
    pub fn get_mut(&mut self) -> &mut Fut {
        &mut self.future
    }

    /// Returns the wrapped future.
    pub fn into_inner(self) -> Fut {
        self.future
    }
}

impl<Fut, RT: Access, T, E> Future for Deadline<Fut, RT>
where
    Fut: Future<Output = Result<T, E>>,
    E: From<DeadlinePassed>,
{
    type Output = Result<T, E>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the future.
        let future = unsafe { Pin::map_unchecked_mut(self.as_mut(), |this| &mut this.future) };
        match future.poll(ctx) {
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => {
                // SAFETY: not moving the timer.
                let timer = unsafe { Pin::map_unchecked_mut(self, |this| &mut this.timer) };
                match timer.poll(ctx) {
                    Poll::Ready(deadline) => Poll::Ready(Err(deadline.into())),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

impl<Fut: Unpin, RT: Access> Unpin for Deadline<Fut, RT> {}

/// An [`AsyncIterator`] that yields an item after an interval has passed.
///
/// This itertor will never return `None`, it will always set another deadline
/// and yield another item after the deadline has passed.
///
/// # Notes
///
/// The next deadline will always will be set for exactly the specified interval
/// after the last passed deadline. This means that if the iterator is not
/// polled often enoguh it can be that deadlines will be set that expire
/// immediately, yielding items in quick succession.
///
/// If the above description behaviour is not desired, but you rather wait a
/// certain interval between work consider using a [`Timer`].
///
/// # Examples
///
/// The following example will print hello world (roughly) every 200
/// milliseconds.
///
/// ```
/// # #![feature(never_type)]
/// #
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::actor::actor_fn;
/// # use heph::supervisor::NoSupervisor;
/// # use heph_rt::spawn::ActorOptions;
/// # use heph_rt::{self as rt, Runtime, RuntimeRef};
/// use heph_rt::ThreadLocal;
/// use heph_rt::timer::Interval;
/// use heph_rt::util::next;
/// #
/// # fn main() -> Result<(), rt::Error> {
/// #     let mut runtime = Runtime::new()?;
/// #     runtime.run_on_workers(setup)?;
/// #     runtime.start()
/// # }
/// #
/// # fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
/// #   let actor = actor_fn(actor);
/// #   let options = ActorOptions::default();
/// #   runtime_ref.spawn_local(NoSupervisor, actor, (), options);
/// #   Ok(())
/// # }
///
/// async fn actor(ctx: actor::Context<!, ThreadLocal>) {
/// #   let start = Instant::now();
///     let mut interval = Interval::every(ctx.runtime_ref().clone(), Duration::from_millis(200));
/// #   assert!(interval.next_deadline() >= start + Duration::from_millis(200));
///     loop {
///         // Wait until the next timer expires.
///         let _ = next(&mut interval).await;
/// #       assert!(start.elapsed() >= Duration::from_millis(200));
///         println!("Hello world");
/// #       break;
///     }
/// }
/// ```
#[derive(Debug)]
#[must_use = "AsyncIterators do nothing unless polled"]
pub struct Interval<RT: Access> {
    timer: Timer<RT>,
    interval: Duration,
}

impl<RT: Access> Interval<RT> {
    /// Create a new `Interval`.
    pub fn every(rt: RT, interval: Duration) -> Interval<RT> {
        Interval {
            interval,
            timer: Timer::after(rt, interval),
        }
    }

    /// Returns the next deadline for this `Interval`.
    pub const fn next_deadline(&self) -> Instant {
        self.timer.deadline
    }
}

impl<RT: Access> AsyncIterator for Interval<RT> {
    type Item = DeadlinePassed;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::into_inner(self);
        match Pin::new(&mut this.timer).poll(ctx) {
            Poll::Ready(deadline) => {
                this.timer.deadline += this.interval;
                this.timer.timer_pending = None;
                Poll::Ready(Some(deadline))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<RT: Access> Unpin for Interval<RT> {}
