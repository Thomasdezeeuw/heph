//! Testing utilities.

// Not all test files use all functions/macros.
#![allow(unused_macros, dead_code)]

use std::lazy::SyncLazy;
use std::sync::Mutex;

// NOTE: keep in sync with the actual capacity.
pub const SMALL_CAP: usize = 8;

/// To not create too many threads concurrently this lock is used to run the
/// tests using `start_threads` sequentially.
pub static THREAD_LOCK: SyncLazy<Mutex<()>> = SyncLazy::new(|| Mutex::new(()));

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
            #[track_caller]
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
        let mut range = (0..500_000);
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
                Err(inbox::SendError::Full(v)) => {
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
                Err(inbox::RecvError::Empty) => {} // Try again.
                Err(err) => panic!("unexpected error receiving: {}", err),
            }
        }
    }};
}
