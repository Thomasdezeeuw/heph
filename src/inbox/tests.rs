use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use crate::inbox::{Inbox, InboxRef};
use crate::rt::ProcessId;

const PID: ProcessId = ProcessId(0);

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

/// Sleep for a minimum amount of time.
///
/// Used in test the only need another thread to run for a moment, e.g. for a
/// type to be dropped.
fn min_sleep() {
    sleep(Duration::from_micros(100));
}

/// Message type used in drop tests.
#[derive(Debug)]
struct DropTest(Arc<AtomicUsize>);

impl DropTest {
    /// Create a new `DropTest` message.
    pub fn new() -> (DropTest, IsDropped) {
        let shared = Arc::new(AtomicUsize::new(0));
        (DropTest(shared.clone()), IsDropped(shared))
    }
}

impl Drop for DropTest {
    fn drop(&mut self) {
        let _ = self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// Type that checks if `DropTest` is actually dropped once and only once.
#[derive(Debug)]
struct IsDropped(Arc<AtomicUsize>);

impl Drop for IsDropped {
    fn drop(&mut self) {
        let dropped = self.0.load(Ordering::Acquire);
        if dropped != 1 {
            panic!("Dropped item {} times", dropped);
        }
    }
}

/// Message type used in drop tests to ensure we don't drop uninitialised memory.
#[derive(Debug)]
struct NeverDrop;

impl Drop for NeverDrop {
    fn drop(&mut self) {
        panic!("Dropped uninitialised memory!");
    }
}

#[test]
fn inbox_is_send() {
    assert_send::<Inbox<()>>();
}

#[test]
fn inbox_is_sync() {
    assert_sync::<Inbox<()>>();
}

#[test]
fn inbox_ref_is_send() {
    assert_send::<InboxRef<()>>();
}

#[test]
fn inbox_ref_is_sync() {
    assert_sync::<InboxRef<()>>();
}

mod oneshot {
    //! Tests for the oneshot channel using a single thread.

    use crate::inbox::oneshot::{channel, Receiver, RecvError, Sender};
    use crate::test::new_waker;

    use super::{assert_send, DropTest, NeverDrop, PID};

    #[test]
    fn size() {
        use std::mem::size_of;
        assert_eq!(size_of::<Sender<()>>(), 8);
        assert_eq!(size_of::<super::InboxRef<()>>(), 32);
    }

    #[test]
    fn sender_is_send() {
        assert_send::<Sender<()>>();
    }

    #[test]
    fn receiver_is_send() {
        assert_send::<Receiver<()>>();
    }

    #[test]
    fn empty_sender() {
        let send = Sender::empty();
        assert!(!send.is_connected());
        assert!(send.try_send(123usize).is_err());
    }

    #[test]
    fn single_send_recv() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        send.try_send(1).unwrap();
        assert_eq!(recv.try_recv().unwrap().0, 1);
    }

    #[test]
    fn zero_sized_types() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        send.try_send(()).unwrap();
        assert_eq!(recv.try_recv().unwrap().0, ());
    }

    #[test]
    fn sending_wakes_waker() {
        const TOKEN: mio::Token = mio::Token(0);
        let poll = mio::Poll::new().unwrap();
        let mio_waker = mio::Waker::new(poll.registry(), TOKEN).unwrap();
        let (wake_send, wake_recv) = crossbeam_channel::bounded(2);
        let waker_id = crate::rt::waker::init(mio_waker, wake_send);
        let waker = crate::rt::waker::Waker::new(waker_id, PID);

        let (send, mut recv) = channel(waker);

        send.try_send(1).unwrap();
        assert_eq!(recv.try_recv().unwrap().0, 1);

        // Ensure `Waker::wake` is called.
        assert_eq!(wake_recv.try_recv().unwrap(), PID);
    }

    #[test]
    fn send_peek() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        send.try_send(1).unwrap();
        assert_eq!(recv.try_peek().unwrap(), 1);
        assert_eq!(recv.try_recv().unwrap().0, 1);
    }

    #[test]
    fn send_double_peek() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        send.try_send(1).unwrap();
        assert_eq!(recv.try_peek().unwrap(), 1);
        assert_eq!(recv.try_peek().unwrap(), 1);
        assert_eq!(recv.try_recv().unwrap().0, 1);
    }

    #[test]
    fn peek_zero_sized_types() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        send.try_send(()).unwrap();
        assert_eq!(recv.try_peek().unwrap(), ());
        assert_eq!(recv.try_peek().unwrap(), ());
        assert_eq!(recv.try_recv().unwrap().0, ());
    }

    #[test]
    fn recv_no_value() {
        let waker = new_waker(PID);
        let (_send, mut recv) = channel::<usize>(waker);

        match recv.try_recv() {
            Ok(_) => panic!("unexpected receive of value"),
            Err(err) => assert_eq!(err, RecvError::NoValue),
        }
    }

    #[test]
    fn recv_no_sender() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel::<usize>(waker);
        drop(send);

        match recv.try_recv() {
            Ok(_) => panic!("unexpected receive of value"),
            Err(err) => assert_eq!(err, RecvError::Disconnected),
        }
    }

    #[test]
    fn peek_no_value() {
        let waker = new_waker(PID);
        let (_send, recv) = channel::<usize>(waker);

        match recv.try_peek() {
            Ok(_) => panic!("unexpected peek of value"),
            Err(err) => assert_eq!(err, RecvError::NoValue),
        }
    }

    #[test]
    fn peek_no_sender() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<usize>(waker);
        drop(send);

        match recv.try_peek() {
            Ok(_) => panic!("unexpected peek of value"),
            Err(err) => assert_eq!(err, RecvError::Disconnected),
        }
    }

    #[test]
    fn send_no_receiver() {
        let waker = new_waker(PID);
        let (send, recv) = channel(waker);
        drop(recv);
        assert!(send.try_send(1).is_err());
    }

    #[test]
    fn sender_is_connected() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<()>(waker);
        assert!(send.is_connected());

        // After a disconnect.
        drop(recv);
        assert!(!send.is_connected());
    }

    #[test]
    fn receiver_is_connected() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<()>(waker);
        assert!(recv.is_connected());

        // After the sender is dropped.
        drop(send);
        assert!(!recv.is_connected());
    }

    #[test]
    fn sender_usage_after_try_recv() {
        let waker = new_waker(PID);
        let (send1, mut recv) = channel(waker);

        // First `try_recv`.
        send1.try_send(123).unwrap();
        let (msg, send2) = recv.try_recv().unwrap();
        assert_eq!(msg, 123);
        assert!(send2.is_connected());
        assert!(recv.is_connected());

        // Usage of the reset `Sender`.
        send2.try_send(456).unwrap();
        let msg = recv.try_peek().unwrap();
        assert_eq!(msg, 456);
        let (msg, send3) = recv.try_recv().unwrap();
        assert_eq!(msg, 456);
        assert!(send3.is_connected());
        assert!(recv.is_connected());

        drop(send3);
        assert!(!recv.is_connected());
        let _send4 = recv.try_reset().unwrap();
    }

    #[test]
    fn sender_usage_after_reset() {
        let waker = new_waker(PID);
        let (send1, mut recv) = channel(waker);
        drop(send1);

        // First `reset`.
        let send2 = recv.try_reset().unwrap();
        assert!(send2.is_connected());
        assert!(recv.is_connected());

        // Usage of the reset `Sender`.
        send2.try_send(123).unwrap();
        let msg = recv.try_peek().unwrap();
        assert_eq!(msg, 123);
        let (msg, send3) = recv.try_recv().unwrap();
        assert_eq!(msg, 123);
        assert!(send3.is_connected());
        assert!(recv.is_connected());
    }

    #[test]
    fn new_receiver_connected() {
        let waker = new_waker(PID);
        let (mut send, recv) = channel::<usize>(waker.clone());
        assert!(send.new_receiver(waker).is_none());
        drop(recv); // Must live until here.
    }

    #[test]
    #[ignore = "Not implemented"]
    fn new_receiver_disconnected() {
        let waker = new_waker(PID);
        let (mut send, recv) = channel(waker.clone());
        drop(recv);

        let mut recv = send.new_receiver(waker).unwrap();
        send.try_send(123).unwrap();
        assert_eq!(recv.try_recv().unwrap().0, 123);
    }

    #[test]
    fn new_receiver_empty_sender() {
        let waker = new_waker(PID);
        let mut send = Sender::empty();

        let mut recv = send.new_receiver(waker).unwrap();
        send.try_send(123).unwrap();
        assert_eq!(recv.try_recv().unwrap().0, 123);
    }

    // Note: the following test names are postfixed with `sr` or `rs`, which means
    // that the `s`ender or `r`eceiver is dropped first (the other parts of the
    // tests must be the same).
    //
    // TODO: make a macro for this or something.

    #[test]
    fn drop_test_empty_sr() {
        // This mainly shouldn't panic or leak memory, the last one can only be
        // detected by an external program.
        let waker = new_waker(PID);
        let (sender, receiver) = channel::<NeverDrop>(waker);
        drop(sender);
        drop(receiver);
    }

    #[test]
    fn drop_test_empty_rs() {
        let waker = new_waker(PID);
        let (sender, receiver) = channel::<NeverDrop>(waker);
        drop(receiver);
        drop(sender);
    }

    #[test]
    fn drop_test_one_message() {
        let waker = new_waker(PID);
        let (sender, receiver) = channel(waker);

        let (msg, _check) = DropTest::new();
        sender.try_send(msg).unwrap();

        drop(receiver);
    }

    #[test]
    fn drop_test_send_twice() {
        let waker = new_waker(PID);
        let (sender1, mut receiver) = channel(waker);

        let (msg, _check1) = DropTest::new();
        sender1.try_send(msg).unwrap();

        let (msg, sender2) = receiver.try_recv().unwrap();
        drop(msg);

        let (msg, _check2) = DropTest::new();
        sender2.try_send(msg).unwrap();

        drop(receiver);
    }
    #[test]
    fn drop_test_send_and_reset_sr() {
        let waker = new_waker(PID);
        let (sender1, mut receiver) = channel(waker);

        let (msg, _check) = DropTest::new();
        sender1.try_send(msg).unwrap();

        let (msg, sender2) = receiver.try_recv().unwrap();
        drop(msg);

        drop(sender2);
        drop(receiver);
    }

    #[test]
    fn drop_test_send_and_reset_rs() {
        let waker = new_waker(PID);
        let (sender1, mut receiver) = channel(waker);

        let (msg, _check) = DropTest::new();
        sender1.try_send(msg).unwrap();

        let (msg, sender2) = receiver.try_recv().unwrap();
        drop(msg);

        drop(receiver);
        drop(sender2);
    }
}

mod oneshot_threaded {
    //! Tests for the one-shot channel using multiple threads.

    // Miri does not support threading.
    #![cfg(not(miri))]

    use std::sync::{Arc, Mutex};
    use std::thread::sleep;
    use std::time::Duration;

    use crate::inbox::oneshot::{channel, RecvError};
    use crate::test::new_waker;

    use super::{min_sleep, DropTest, NeverDrop, PID};

    /// Start a number of threads and wait for them to finish.
    macro_rules! start_threads {
        ($( $thread: block $(,)* )+) => {{
            let threads = vec![
                $(
                    std::thread::spawn(move || $thread)
                ),+
            ];
            threads
                .into_iter()
                .map(|handle| handle.join())
                .collect::<std::thread::Result<()>>()
                .unwrap();
        }};
    }

    /// Macro to create a not infinite loop.
    ///
    /// This will panic if it did too many iterations.
    macro_rules! r#loop {
        ($($arg: tt)*) => {
            // Don't want to actually loop for ever.
            for i in 0..1000 {
                $($arg)*

                if i == 1000 {
                    panic!("too many iterations");
                }
            }
        }
    }

    /// Loop until a value is received.
    macro_rules! expect_recv {
        ($recv: expr, $expected: expr) => {
            r#loop! {
                match $recv.try_recv() {
                    Ok((msg, _)) => {
                        assert_eq!(msg, $expected);
                        break;
                    }
                    Err(RecvError::NoValue) => {} // Value not ready yet.
                    Err(RecvError::Disconnected) => {
                        panic!("unexpected error receiving: {}", RecvError::Disconnected)
                    }
                }
            }
        };
    }

    macro_rules! expect_peek {
        ($recv: expr, $expected: expr) => {
            r#loop! {
                match $recv.try_peek() {
                    Ok(msg) => {
                        assert_eq!(msg, $expected);
                        break;
                    }
                    Err(RecvError::NoValue) => {} // Value not ready yet.
                    Err(RecvError::Disconnected) => {
                        panic!("unexpected error receiving: {}", RecvError::Disconnected)
                    }
                }
            }
        };
    }

    #[test]
    fn single_send_recv() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(1).unwrap();
            },
            {
                expect_recv!(recv, 1);
            }
        );
    }

    #[test]
    fn zero_sized_types() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(()).unwrap();
            },
            {
                expect_recv!(recv, ());
            }
        );
    }

    #[test]
    fn send_peek() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(1).unwrap();
            },
            {
                expect_peek!(recv, 1);
                assert_eq!(recv.try_recv().unwrap().0, 1);
            }
        );
    }

    #[test]
    fn send_double_peek() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(1).unwrap();
            },
            {
                expect_peek!(recv, 1);
                assert_eq!(recv.try_peek().unwrap(), 1);
                assert_eq!(recv.try_recv().unwrap().0, 1);
            }
        );
    }

    #[test]
    fn peek_zero_sized_types() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(()).unwrap();
            },
            {
                expect_peek!(recv, ());
                assert_eq!(recv.try_peek().unwrap(), ());
                assert_eq!(recv.try_recv().unwrap().0, ());
            }
        );
    }

    #[test]
    fn recv_no_sender() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel::<usize>(waker);

        start_threads!(
            {
                drop(send);
            },
            {
                // Let the receiver be dropped, we don't want to use any barriers
                // here as that would defeat the point of testing our
                // synchronisation.
                min_sleep();
                match recv.try_recv() {
                    Ok(_) => panic!("unexpected receive of value"),
                    Err(err) => assert_eq!(err, RecvError::Disconnected),
                }
            }
        );
    }

    #[test]
    fn peek_no_sender() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<usize>(waker);

        start_threads!(
            {
                drop(send);
            },
            {
                // Let the receiver be dropped, we don't want to use any barriers
                // here as that would defeat the point of testing our
                // synchronisation.
                min_sleep();
                match recv.try_peek() {
                    Ok(_) => panic!("unexpected receive of value"),
                    Err(err) => assert_eq!(err, RecvError::Disconnected),
                }
            }
        );
    }

    #[test]
    fn send_no_receiver() {
        let waker = new_waker(PID);
        let (send, recv) = channel(waker);

        start_threads!(
            {
                min_sleep();
                assert!(send.try_send(1).is_err());
            },
            {
                drop(recv);
            }
        );
    }

    #[test]
    fn sender_is_connected() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<()>(waker);

        start_threads!(
            {
                r#loop! {
                    if !send.is_connected() {
                        break;
                    }
                }
            },
            {
                drop(recv);
            }
        );
    }

    #[test]
    fn receiver_is_connected() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<()>(waker);

        start_threads!(
            {
                drop(send);
            },
            {
                r#loop! {
                    if !recv.is_connected() {
                        break;
                    }
                }
            }
        );
    }

    #[test]
    fn sender_usage_after_try_recv() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                send.try_send(123).unwrap();
            },
            {
                r#loop! {
                    match recv.try_recv() {
                        Ok((msg, send2)) => {
                            assert_eq!(msg, 123);
                            assert!(send2.is_connected());

                            send2.try_send(456).unwrap();
                            assert_eq!(recv.try_peek().unwrap(), 456);
                            let (msg, send3) = recv.try_recv().unwrap();
                            assert_eq!(msg, 456);

                            assert!(send3.is_connected());
                            assert!(recv.is_connected());
                            break;
                        }
                        Err(RecvError::NoValue) => {} // Value not ready yet.
                        Err(RecvError::Disconnected) => {
                            panic!("unexpected error receiving: {}", RecvError::Disconnected)
                        }
                    }
                }
            }
        );
    }

    #[test]
    fn sender_usage_after_reset() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);

        start_threads!(
            {
                drop(send);
            },
            {
                r#loop! {
                    match recv.try_reset() {
                        Ok(send2) => {
                            assert!(send2.is_connected());

                            send2.try_send(123).unwrap();
                            assert_eq!(recv.try_peek().unwrap(), 123);
                            let (msg, send3) = recv.try_recv().unwrap();
                            assert_eq!(msg, 123);

                            assert!(send3.is_connected());
                            assert!(recv.is_connected());
                            break;
                        }
                        Err(()) => {} // Sender still connected.
                    }
                }
            }
        );
    }

    #[test]
    fn drop_test_empty() {
        let waker = new_waker(PID);
        let (send, recv) = channel::<NeverDrop>(waker);

        start_threads!(
            {
                drop(send);
            },
            {
                drop(recv);
            }
        );
    }

    #[test]
    fn drop_test_one_message() {
        let waker = new_waker(PID);
        let (send, recv) = channel(waker);
        let check = Arc::new(Mutex::new(None));
        let set_check = check.clone();

        start_threads!(
            {
                let (msg, check) = DropTest::new();
                send.try_send(msg).unwrap();
                *set_check.lock().unwrap() = Some(check);
            },
            {
                sleep(Duration::from_millis(1));
                drop(recv);
            }
        );
        drop(check);
    }

    #[test]
    fn drop_test_send_twice() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);
        let check1 = Arc::new(Mutex::new(None));
        let set_check1 = check1.clone();
        let check2 = Arc::new(Mutex::new(None));
        let set_check2 = check2.clone();

        start_threads!(
            {
                let (msg, check1) = DropTest::new();
                send.try_send(msg).unwrap();
                *set_check1.lock().unwrap() = Some(check1);
            },
            {
                r#loop! {
                    match recv.try_recv() {
                        Ok((_msg, send)) => {
                            let (msg, check2) = DropTest::new();
                            send.try_send(msg).unwrap();
                            *set_check2.lock().unwrap() = Some(check2);
                            break;
                        }
                        Err(RecvError::NoValue) => {} // Value not ready yet.
                        Err(RecvError::Disconnected) => {
                            panic!("unexpected error receiving: {}", RecvError::Disconnected)
                        }
                    }
                }
            }
        );

        drop(check1);
        drop(check2);
    }

    #[test]
    fn drop_test_send_and_reset() {
        let waker = new_waker(PID);
        let (send, mut recv) = channel(waker);
        let check1 = Arc::new(Mutex::new(None));
        let set_check1 = check1.clone();
        let check2 = Arc::new(Mutex::new(None));
        let set_check2 = check2.clone();

        start_threads!(
            {
                let (msg, check1) = DropTest::new();
                send.try_send(msg).unwrap();
                *set_check1.lock().unwrap() = Some(check1);
            },
            {
                r#loop! {
                    match recv.try_reset() {
                        Ok(send) => {
                            let (msg, check2) = DropTest::new();
                            send.try_send(msg).unwrap();
                            *set_check2.lock().unwrap() = Some(check2);
                            break;
                        }
                        Err(()) => {} // Sender still connected.
                    }
                }
            }
        );

        drop(check1);
        drop(check2);
    }
}
