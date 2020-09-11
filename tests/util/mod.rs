//! Testing utilities.

// Not all test files use all functions/macros.
#![allow(unused_macros, dead_code)]

// NOTE: keep in sync with the actual capacity.
pub const SMALL_CAP: usize = 8;

/// Start a number of threads and wait for them to finish.
macro_rules! start_threads {
    ($( $thread: block $(,)* )+) => {{
        let threads = vec![
            $(
                std::thread::spawn(move || $thread)
            ),+
        ];
        let result = threads
            .into_iter()
            .map(|handle| handle.join())
            .collect::<std::thread::Result<()>>();

        if let Err(err) = result {
            let msg: &str = match err.downcast_ref::<String>() {
                Some(msg) => &*msg,
                None => match err.downcast_ref::<&str>() {
                    Some(msg) => msg,
                    None => "unkown error",
                },
            };
            #[track_caller]
            panic!("thread failed: {}", msg);
        }
    }};
}

/// Macro to create a not infinite loop.
///
/// This will panic if it did too many iterations.
macro_rules! r#loop {
    ($($arg: tt)*) => {{
        // Don't want to actually loop for ever.
        for i in 0..1000 {
            $($arg)*

            if i == 1000 {
                panic!("too many iterations");
            }
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
