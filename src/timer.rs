//! Module with time related utilities.
//!
//! This module provides three types.
//!
//! - [`Timer`](Timer) is a stand-alone future that returns
//!   [`DeadlinePassed`](DeadlinePassed) once the deadline has passed.
//! - [`Deadline`](Deadline) wraps another [`Future`] and checks the deadline
//!   each time it's polled.
//! - [`Interval`](Interval) implements [`Stream`] which yields an item
//!   after the deadline has passed each interval.

use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::stream::Stream;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{io, ptr};

use crate::{actor, rt};

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

/// A [`Future`] that represents a timer.
///
/// If this future returns [`Poll::Ready`]`(`[`DeadlinePassed`]`)` it means that
/// the deadline has passed. If it returns [`Poll::Pending`] it's not yet
/// passed.
///
/// # Examples
///
/// ```
/// #![feature(never_type)]
///
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::supervisor::NoSupervisor;
/// # use heph::{rt, ActorOptions, Runtime, RuntimeRef};
/// use heph::rt::ThreadLocal;
/// use heph::timer::Timer;
///
/// # fn main() -> Result<(), rt::Error> {
/// #     let mut runtime = Runtime::new()?;
/// #     runtime.run_on_workers(setup)?;
/// #     runtime.start()
/// # }
/// #
/// #
/// # fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
/// #   let actor = actor as fn(_) -> _;
/// #   let options = ActorOptions::default();
/// #   runtime_ref.spawn_local(NoSupervisor, actor, (), options);
/// #   Ok(())
/// # }
/// #
/// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
/// #   let start = Instant::now();
///     // Create a timer, this will be ready once the timeout has passed.
///     let timeout = Timer::after(&mut ctx, Duration::from_millis(200));
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
pub struct Timer<RT: rt::Access> {
    deadline: Instant,
    rt: RT,
    // NOTE: when adding fields also add to [`Timer::wrap`].
}

impl<RT: rt::Access> Timer<RT> {
    /// Create a new `Timer`.
    ///
    /// # Panics
    ///
    /// This will panic if `deadline` is in the past.
    pub fn at<M>(ctx: &mut actor::Context<M, RT>, deadline: Instant) -> Timer<RT>
    where
        RT: Clone,
    {
        let mut rt = ctx.runtime().clone();
        rt.add_deadline(deadline);
        Timer { deadline, rt }
    }

    /// Create a new timer, based on a timeout.
    ///
    /// Same as calling `Timer::at(&mut ctx, Instant::now() + timeout)`.
    pub fn after<M>(ctx: &mut actor::Context<M, RT>, timeout: Duration) -> Timer<RT>
    where
        RT: Clone,
    {
        Timer::at(ctx, Instant::now() + timeout)
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
    pub fn wrap<Fut>(self, future: Fut) -> Deadline<Fut, RT> {
        // We don't want to run the destructor as that would remove the
        // deadline, which we need in `Deadline` as well. As a bonus we can
        // safetly move `RT` without having to clone it (which normally can't be
        // done with `Drop` types).
        // Safety: See [`ManuallyDrop::take`], rather then taking the entire
        // thing struct at once we read (move out of) value by value.
        let this = ManuallyDrop::new(self);
        let deadline = unsafe { ptr::addr_of!(this.deadline).read() };
        let rt = unsafe { ptr::addr_of!(this.rt).read() };
        Deadline {
            deadline,
            future,
            rt,
        }
    }
}

impl<RT: rt::Access> Future for Timer<RT> {
    type Output = DeadlinePassed;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.has_passed() {
            Poll::Ready(DeadlinePassed)
        } else {
            Poll::Pending
        }
    }
}

impl<RT: rt::Access> Unpin for Timer<RT> {}

impl<RT: rt::Access> actor::Bound<RT> for Timer<RT> {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        let old_pid = self.rt.change_pid(ctx.runtime_ref().pid());
        self.rt.change_deadline(old_pid, self.deadline);
        Ok(())
    }
}

impl<RT: rt::Access> Drop for Timer<RT> {
    fn drop(&mut self) {
        self.rt.remove_deadline(self.deadline);
    }
}

/// A [`Future`] that wraps another future setting a deadline for it.
///
/// When this future is polled it first checks if the deadline has passed, if so
/// it returns [`Poll::Ready`]`(Err(`[`DeadlinePassed`]`.into()))`. Otherwise
/// this will poll the future it wraps.
///
/// # Notes
///
/// This type can also be created using [`Timer::wrap`], this is useful when
/// dealing with lifetime issue, e.g. when calling
/// [`actor::Context::receive_next`] and wrapping that in a `Deadline`.
///
/// # Examples
///
/// Setting a timeout for a future.
///
/// ```
/// #![feature(never_type)]
///
/// use std::io;
/// # use std::future::Future;
/// # use std::pin::Pin;
/// # use std::task::{self, Poll};
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::supervisor::NoSupervisor;
/// use heph::rt::ThreadSafe;
/// # use heph::{rt, ActorOptions, Runtime};
/// use heph::timer::Deadline;
///
/// # fn main() -> Result<(), rt::Error> {
/// #     let actor = actor as fn(_) -> _;
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
/// async fn actor(mut ctx: actor::Context<String, ThreadSafe>) {
///     // `OtherFuture` is a type that implements `Future`.
///     let future = IoFuture;
///     // Create our deadline.
/// #   let start = Instant::now();
///     let deadline_future = Deadline::after(&mut ctx, Duration::from_millis(100), future);
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
pub struct Deadline<Fut, RT: rt::Access> {
    deadline: Instant,
    future: Fut,
    rt: RT,
    // NOTE: when adding fields also add to [`Deadline::into_inner`].
}

impl<Fut, RT: rt::Access> Deadline<Fut, RT> {
    /// Create a new `Deadline`.
    ///
    /// # Panics
    ///
    /// This will panic if `deadline` is in the past.
    pub fn at<M>(
        ctx: &mut actor::Context<M, RT>,
        deadline: Instant,
        future: Fut,
    ) -> Deadline<Fut, RT>
    where
        RT: Clone,
    {
        let mut rt = ctx.runtime().clone();
        rt.add_deadline(deadline);
        Deadline {
            deadline,
            future,
            rt,
        }
    }

    /// Create a new deadline based on a timeout.
    ///
    /// Same as calling `Deadline::at(&mut ctx, Instant::now() + timeout,
    /// future)`.
    pub fn after<M>(
        ctx: &mut actor::Context<M, RT>,
        timeout: Duration,
        future: Fut,
    ) -> Deadline<Fut, RT>
    where
        RT: Clone,
    {
        Deadline::at(ctx, Instant::now() + timeout, future)
    }

    /// Returns the deadline set for this `Deadline`.
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns `true` if the deadline has passed.
    pub fn has_passed(&self) -> bool {
        self.deadline <= Instant::now()
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
    pub fn into_inner(mut self) -> Fut {
        self.rt.remove_deadline(self.deadline);
        // Safety: See [`ManuallyDrop::take`], rather then taking the entire
        // thing struct at once we read (move out of) value by value.
        let mut this = ManuallyDrop::new(self);
        unsafe { ptr::addr_of_mut!(this.deadline).drop_in_place() }
        unsafe { ptr::addr_of_mut!(this.rt).drop_in_place() }
        unsafe { ptr::addr_of!(this.future).read() }
    }
}

/* TODO: add this once `specialization` feature is stabilised.
impl<Fut> Future for Deadline<Fut>
where
    Fut: Future,
{
    type Output = Result<Fut::Output, DeadlinePassed>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.has_passed() {
            Poll::Ready(Err(DeadlinePassed))
        } else {
            // Safety: this is safe because we're not moving the future.
            let future = unsafe {
                Pin::map_unchecked_mut(self, |this| &mut this.future)
            };
            future.poll(ctx).map(Ok)
        }
    }
}
*/

impl<Fut, RT: rt::Access, T, E> Future for Deadline<Fut, RT>
where
    Fut: Future<Output = Result<T, E>>,
    E: From<DeadlinePassed>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.has_passed() {
            Poll::Ready(Err(DeadlinePassed.into()))
        } else {
            // Safety: this is safe because we're not moving the future.
            let future = unsafe { Pin::map_unchecked_mut(self, |this| &mut this.future) };
            future.poll(ctx)
        }
    }
}

impl<Fut: Unpin, RT: rt::Access> Unpin for Deadline<Fut, RT> {}

impl<Fut, RT: rt::Access> actor::Bound<RT> for Deadline<Fut, RT> {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        let old_pid = self.rt.change_pid(ctx.runtime_ref().pid());
        self.rt.change_deadline(old_pid, self.deadline);
        Ok(())
    }
}

impl<Fut, RT: rt::Access> Drop for Deadline<Fut, RT> {
    fn drop(&mut self) {
        self.rt.remove_deadline(self.deadline);
    }
}

/// A [`Stream`] that yields an item after an interval has passed.
///
/// This stream will never return `None`, it will always set another deadline
/// and yield another item after the deadline has passed.
///
/// # Notes
///
/// The next deadline will always will be set after this returns `Poll::Ready`.
/// This means that if the interval is very short and the stream is not polled
/// often enough it's possible that the actual time between yielding two values
/// can become bigger then the specified interval.
///
/// # Examples
///
/// The following example will print hello world (roughly) every 200
/// milliseconds.
///
/// ```
/// #![feature(never_type)]
///
/// use std::time::Duration;
/// # use std::time::Instant;
///
/// use heph::actor;
/// # use heph::supervisor::NoSupervisor;
/// # use heph::{rt, ActorOptions, Runtime, RuntimeRef};
/// use heph::rt::ThreadLocal;
/// use heph::timer::Interval;
/// use heph::util::next;
/// #
/// # fn main() -> Result<(), rt::Error> {
/// #     let mut runtime = Runtime::new()?;
/// #     runtime.run_on_workers(setup)?;
/// #     runtime.start()
/// # }
/// #
/// # fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
/// #   let actor = actor as fn(_) -> _;
/// #   let options = ActorOptions::default();
/// #   runtime_ref.spawn_local(NoSupervisor, actor, (), options);
/// #   Ok(())
/// # }
///
/// async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
/// #   let start = Instant::now();
///     let mut interval = Interval::every(&mut ctx, Duration::from_millis(200));
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
#[must_use = "streams do nothing unless polled"]
pub struct Interval<RT: rt::Access> {
    deadline: Instant,
    interval: Duration,
    rt: RT,
}

impl<RT: rt::Access> Interval<RT> {
    /// Create a new `Interval`.
    pub fn every<M>(ctx: &mut actor::Context<M, RT>, interval: Duration) -> Interval<RT>
    where
        RT: Clone,
    {
        let deadline = Instant::now() + interval;
        let mut rt = ctx.runtime().clone();
        rt.add_deadline(deadline);
        Interval {
            deadline,
            interval,
            rt,
        }
    }

    /// Returns the next deadline for this `Interval`.
    pub const fn next_deadline(&self) -> Instant {
        self.deadline
    }
}

impl<RT: rt::Access> Stream for Interval<RT> {
    type Item = DeadlinePassed;

    fn poll_next(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        if self.deadline <= Instant::now() {
            // Determine the next deadline.
            let next_deadline = Instant::now() + self.interval;
            let this = Pin::get_mut(self);
            this.deadline = next_deadline;
            this.rt.add_deadline(next_deadline);
            Poll::Ready(Some(DeadlinePassed))
        } else {
            Poll::Pending
        }
    }
}

impl<RT: rt::Access> Unpin for Interval<RT> {}

impl<RT: rt::Access> actor::Bound<RT> for Interval<RT> {
    type Error = !;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> Result<(), !> {
        let old_pid = self.rt.change_pid(ctx.runtime_ref().pid());
        self.rt.change_deadline(old_pid, self.deadline);
        Ok(())
    }
}

impl<RT: rt::Access> Drop for Interval<RT> {
    fn drop(&mut self) {
        self.rt.remove_deadline(self.deadline);
    }
}
