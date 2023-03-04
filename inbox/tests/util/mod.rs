//! Testing utilities.

// Not all test files use all functions/macros.
#![allow(unused_macros, dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{self, Wake};
use std::thread;

pub fn assert_send<T: Send>() {}
pub fn assert_sync<T: Sync>() {}

/// Number of times the waker was awoken.
///
/// See [`new_count_waker`] for usage.
#[derive(Debug)]
pub struct AwokenCount {
    inner: Arc<WakerInner>,
}

impl AwokenCount {
    /// Get the current count.
    pub fn get(&self) -> usize {
        self.inner.count.load(Ordering::SeqCst)
    }
}

impl PartialEq<usize> for AwokenCount {
    fn eq(&self, other: &usize) -> bool {
        self.get() == *other
    }
}

#[derive(Debug)]
struct WakerInner {
    count: AtomicUsize,
}

impl Wake for WakerInner {
    fn wake(self: Arc<Self>) {
        let _ = self.count.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.count.fetch_add(1, Ordering::SeqCst);
    }
}

/// Create a new [`Waker`] that counts the number of times it's awoken.
pub fn new_count_waker() -> (task::Waker, AwokenCount) {
    let inner = Arc::new(WakerInner {
        count: AtomicUsize::new(0),
    });
    (inner.clone().into(), AwokenCount { inner })
}

/// Message type used in drop tests to ensure we don't drop undefined memory.
#[derive(Debug)]
pub struct NeverDrop;

impl Drop for NeverDrop {
    fn drop(&mut self) {
        panic!("Dropped uninitialised memory!");
    }
}

/// Message type used in drop tests.
#[derive(Debug)]
pub struct DropTest(Arc<AtomicUsize>);

impl DropTest {
    /// Create a new `DropTest` message.
    pub fn new() -> (DropTest, IsDropped) {
        let shared = Arc::new(AtomicUsize::new(0));
        (DropTest(shared.clone()), IsDropped(shared))
    }

    /// Returns two vectors of `DropTest`s and `IsDropped` checks, both of
    /// length `n`.
    pub fn many(n: usize) -> (Vec<DropTest>, Vec<IsDropped>) {
        let mut values = Vec::with_capacity(n);
        let mut checks = Vec::with_capacity(n);
        for _ in 0..n {
            let (value, check) = DropTest::new();
            values.push(value);
            checks.push(check);
        }
        (values, checks)
    }
}

impl Drop for DropTest {
    fn drop(&mut self) {
        let _ = self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// Type that checks if `DropTest` is actually dropped once and only once.
#[derive(Debug)]
pub struct IsDropped(Arc<AtomicUsize>);

impl Drop for IsDropped {
    fn drop(&mut self) {
        let dropped = self.0.load(Ordering::Acquire);
        if dropped != 1 {
            if thread::panicking() {
                eprintln!("Dropped item {} times, but already panicking", dropped);
            } else {
                panic!("Dropped item {} times", dropped);
            }
        }
    }
}

/// To not create too many threads concurrently this lock is used to run the
/// tests using `start_threads` sequentially.
pub static THREAD_LOCK: Mutex<()> = Mutex::new(());

/// Start a number of threads and wait for them to finish.
macro_rules! start_threads {
    ($( $thread: block $(,)* )+) => {{
        let n = start_threads!(__count $( $thread, )+);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(n));
        let thread_guard = crate::util::THREAD_LOCK.lock().unwrap();
        let handles = vec![
            $({
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    $thread
                })
            }),+
        ];
        drop(barrier);

        let mut errors = Vec::new();
        for handle in handles {
            if let Err(err) = handle.join() {
                let msg: String = match err.downcast_ref::<String>() {
                    Some(msg) => msg.clone(),
                    None => match err.downcast_ref::<&str>() {
                        Some(msg) => (*msg).to_owned(),
                        None => "unkown error".into(),
                    },
                };
                errors.push(msg);
            }
        }

        // Don't poison the lock.
        drop(thread_guard);

        if !errors.is_empty() {
            panic!("thread failed, error(s): {:?}", errors);
        }
    }};
    (__count $t1: block, $( $thread: block $(,)* )+) => {
        1 + start_threads!(__count $( $thread, )+)
    };
    (__count $t1: block $(,)* ) => { 1 };
}

/// Macro to create a not infinite loop.
///
/// This will panic if it did too many iterations.
macro_rules! r#loop {
    ($($arg: tt)*) => {{
        // Don't want to actually loop for ever.
        let mut range = (0..100_000_000);
        while range.next().is_some() {
            $($arg)*
        }

        if range.is_empty() {
            panic!("looped too many iterations");
        }
    }}
}

/// Loop until a value is send.
macro_rules! expect_send {
    ($sender: expr, $value: expr) => {{
        let mut value = $value;
        r#loop! {
            match $sender.try_send(value) {
                Ok(()) => break,
                Err(heph_inbox::SendError::Full(v)) => {
                    // Try again.
                    value = v;
                },
                Err(err) => panic!("unexpected error sending: {}", err),
            }
        }
    }};
}

/// Loop until a value is received.
macro_rules! expect_recv {
    ($receiver: expr, $expected: expr) => {{
        r#loop! {
            match $receiver.try_recv() {
                Ok(msg) => {
                    assert_eq!(msg, $expected);
                    break;
                }
                Err(heph_inbox::RecvError::Empty) => {} // Try again.
                Err(err) => panic!("unexpected error receiving: {}", err),
            }
        }
    }};
}

/// Loop until a value is peeked.
macro_rules! expect_peek {
    ($receiver: expr, $expected: expr) => {{
        r#loop! {
            match $receiver.try_peek() {
                Ok(msg) => {
                    assert_eq!(*msg, $expected);
                    break;
                }
                Err(heph_inbox::RecvError::Empty) => {} // Try again.
                Err(err) => panic!("unexpected error receiving: {}", err),
            }
        }
    }};
}

/// Run a test with all possible capacities.
macro_rules! with_all_capacities {
    (|$capacity: ident| $test: block) => {{
        for $capacity in inbox::MIN_CAP..=inbox::MAX_CAP {
            $test
        }
    }};
}
