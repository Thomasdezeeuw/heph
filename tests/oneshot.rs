//! Tests for the oneshot channel.

#![feature(once_cell)]

#[macro_use]
mod util;

mod functional {
    use heph_inbox::oneshot::{new_oneshot, Receiver, RecvError, Sender};

    use crate::util::{assert_send, assert_sync};

    #[test]
    fn sender_is_send() {
        assert_send::<Sender<()>>();
    }

    #[test]
    fn sender_is_sync() {
        assert_sync::<Sender<()>>();
    }

    #[test]
    fn receiver_is_send() {
        assert_send::<Receiver<()>>();
    }

    #[test]
    fn receiver_is_sync() {
        assert_sync::<Receiver<()>>();
    }

    #[test]
    fn single_send_recv() {
        let (sender, mut receiver) = new_oneshot();
        sender.try_send(1).unwrap();
        assert_eq!(receiver.try_recv().unwrap().0, 1);
    }

    #[test]
    fn zero_sized_types() {
        let (sender, mut receiver) = new_oneshot();
        sender.try_send(()).unwrap();
        receiver.try_recv().unwrap();
    }

    #[test]
    fn send_no_receiver() {
        let (sender, receiver) = new_oneshot();
        drop(receiver);
        assert_eq!(sender.try_send(1).unwrap_err(), 1);
    }

    #[test]
    fn receive_no_value() {
        let (sender, mut receiver) = new_oneshot::<usize>();
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::NoValue);
        drop(sender);
    }

    #[test]
    fn receive_no_sender() {
        let (sender, mut receiver) = new_oneshot::<()>();
        drop(sender);
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    }

    #[test]
    fn sender_is_connected() {
        let (sender, receiver) = new_oneshot::<()>();
        assert_eq!(sender.is_connected(), true);

        drop(receiver);
        assert_eq!(sender.is_connected(), false);
    }

    #[test]
    fn receiver_is_connected() {
        let (sender, receiver) = new_oneshot::<()>();
        assert_eq!(receiver.is_connected(), true);

        drop(sender);
        assert_eq!(receiver.is_connected(), false);
    }

    #[test]
    fn sender_sends_to() {
        let (sender, receiver) = new_oneshot::<()>();
        let (sender2, receiver2) = new_oneshot::<()>();

        assert_eq!(sender.sends_to(&receiver), true);
        assert_eq!(sender2.sends_to(&receiver2), true);
        assert_eq!(sender.sends_to(&receiver2), false);
        assert_eq!(sender2.sends_to(&receiver), false);
    }
}

mod future {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use futures_test::task::new_count_waker;

    use heph_inbox::oneshot::new_oneshot;

    macro_rules! pin_stack {
        ($fut: ident) => {
            let mut $fut = $fut;
            #[allow(unused_mut)]
            let mut $fut = unsafe { Pin::new_unchecked(&mut $fut) };
        };
    }

    #[test]
    fn sending_wakes_receiver() {
        let (sender, mut receiver) = new_oneshot::<usize>();

        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx).is_pending());
        assert_eq!(count, 0);

        sender.try_send(1).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(Some(1))
        );
    }

    #[test]
    fn dropping_sender_wakes_receiver() {
        let (sender, mut receiver) = new_oneshot::<usize>();

        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx).is_pending());
        assert_eq!(count, 0);

        drop(sender);
        assert_eq!(count, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(None)
        );
    }

    #[test]
    fn sending_wakes_receiver_different_waker() {
        let (sender, mut receiver) = new_oneshot::<usize>();

        let (waker1, count1) = new_count_waker();
        let mut ctx1 = task::Context::from_waker(&waker1);
        let (waker2, count2) = new_count_waker();
        let mut ctx2 = task::Context::from_waker(&waker2);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx1).is_pending());
        assert!(future.as_mut().poll(&mut ctx2).is_pending());
        assert_eq!(count1, 0);
        assert_eq!(count2, 0);

        sender.try_send(1).unwrap();
        assert_eq!(count1, 0);
        assert_eq!(count2, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx2).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(Some(1))
        );
    }

    #[test]
    fn dropping_sender_wakes_receiver_different_waker() {
        let (sender, mut receiver) = new_oneshot::<usize>();

        let (waker1, count1) = new_count_waker();
        let mut ctx1 = task::Context::from_waker(&waker1);
        let (waker2, count2) = new_count_waker();
        let mut ctx2 = task::Context::from_waker(&waker2);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx1).is_pending());
        assert!(future.as_mut().poll(&mut ctx2).is_pending());
        assert_eq!(count1, 0);
        assert_eq!(count2, 0);

        drop(sender);
        assert_eq!(count1, 0);
        assert_eq!(count2, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx2).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(None)
        );
    }

    #[test]
    fn sending_wakes_receiver_after_reset() {
        let (sender, mut receiver) = new_oneshot::<usize>();
        drop(sender);
        let sender = receiver.try_reset().unwrap();

        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx).is_pending());
        assert_eq!(count, 0);

        sender.try_send(1).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(Some(1))
        );
    }

    #[test]
    fn dropping_sender_wakes_receiver_after_reset() {
        let (sender, mut receiver) = new_oneshot::<usize>();
        drop(sender);
        let sender = receiver.try_reset().unwrap();

        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        let future = receiver.recv();
        pin_stack!(future);

        assert!(future.as_mut().poll(&mut ctx).is_pending());
        assert_eq!(count, 0);

        drop(sender);
        assert_eq!(count, 1);
        assert_eq!(
            future.as_mut().poll(&mut ctx).map(|ok| ok.map(|v| v.0)),
            Poll::Ready(None)
        );
    }
}

mod drop {
    use heph_inbox::oneshot::new_oneshot;

    use crate::util::{DropTest, NeverDrop};

    #[test]
    fn empty() {
        let (sender, receiver) = new_oneshot::<NeverDrop>();
        drop(sender);
        drop(receiver);

        let (sender, receiver) = new_oneshot::<NeverDrop>();
        drop(sender);
        drop(receiver);
    }

    #[test]
    fn send_value() {
        let (sender, receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(receiver);
    }

    #[test]
    fn value_send_and_received() {
        let (sender, mut receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn value_send_and_received_and_send() {
        let (sender, mut receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        let (value, sender2) = receiver.try_recv().unwrap();
        let (value2, _check) = DropTest::new();
        sender2.try_send(value2).unwrap();
        drop(value);
        drop(receiver);
    }

    #[test]
    fn value_send_and_received_and_send_received() {
        let (sender, mut receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        let (value, sender2) = receiver.try_recv().unwrap();
        let (value2, _check) = DropTest::new();
        sender2.try_send(value2).unwrap();
        assert!(receiver.try_recv().is_ok());
        drop(value);
    }

    #[test]
    fn reset_sr() {
        let (sender, mut receiver) = new_oneshot::<NeverDrop>();
        drop(sender);
        let sender2 = receiver.try_reset().unwrap();
        drop(sender2);
        drop(receiver);
    }

    #[test]
    fn reset_rs() {
        let (sender, mut receiver) = new_oneshot::<NeverDrop>();
        drop(sender);
        let sender2 = receiver.try_reset().unwrap();
        drop(sender2);
        drop(receiver);
    }

    #[test]
    fn send_and_reset_sr() {
        let (sender, mut receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        let sender2 = receiver.try_reset().unwrap();
        drop(sender2);
        drop(receiver);
    }

    #[test]
    fn send_and_reset_rs() {
        let (sender, mut receiver) = new_oneshot();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        let sender2 = receiver.try_reset().unwrap();
        drop(receiver);
        drop(sender2);
    }

    mod threaded {
        use std::thread;
        use std::time::Duration;

        use heph_inbox::oneshot::new_oneshot;

        use crate::util::{DropTest, NeverDrop};

        #[test]
        fn empty() {
            let (sender, receiver) = new_oneshot::<NeverDrop>();

            start_threads!(
                {
                    drop(sender);
                },
                {
                    drop(receiver);
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn send_value() {
            let (sender, receiver) = new_oneshot();
            let (value, _check) = DropTest::new();

            start_threads!(
                {
                    sender.try_send(value).unwrap();
                },
                {
                    // Give the sender a chance to send the message.
                    thread::sleep(Duration::from_millis(10));
                    drop(receiver);
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn value_send_and_received() {
            let (sender, mut receiver) = new_oneshot();
            let (value, _check) = DropTest::new();

            start_threads!(
                {
                    sender.try_send(value).unwrap();
                },
                {
                    // Give the sender a chance to send the message.
                    thread::sleep(Duration::from_millis(10));
                    assert!(receiver.try_recv().is_ok());
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn value_send_and_received_and_send() {
            let (sender, mut receiver) = new_oneshot();
            let (value, _check) = DropTest::new();

            start_threads!(
                {
                    sender.try_send(value).unwrap();
                },
                {
                    // Give the sender a chance to send the message.
                    thread::sleep(Duration::from_millis(10));
                    let (value, sender2) = receiver.try_recv().unwrap();
                    let (value2, _check) = DropTest::new();
                    sender2.try_send(value2).unwrap();
                    drop(value);
                    drop(receiver);
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn value_send_and_received_and_send_received() {
            let (sender, mut receiver) = new_oneshot();
            let (value, _check) = DropTest::new();

            start_threads!(
                {
                    sender.try_send(value).unwrap();
                },
                {
                    // Give the sender a chance to send the message.
                    thread::sleep(Duration::from_millis(10));
                    let (value, sender2) = receiver.try_recv().unwrap();
                    let (value2, _check) = DropTest::new();
                    sender2.try_send(value2).unwrap();
                    assert!(receiver.try_recv().is_ok());
                    drop(value);
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn reset() {
            let (sender, mut receiver) = new_oneshot::<NeverDrop>();

            start_threads!(
                {
                    drop(sender);
                },
                {
                    // Give the sender a chance to be dropped.
                    thread::sleep(Duration::from_millis(10));
                    let sender2 = receiver.try_reset().unwrap();
                    drop(sender2);
                    drop(receiver);
                }
            );
        }

        #[test]
        fn reset_rs() {
            let (sender, mut receiver) = new_oneshot::<NeverDrop>();
            drop(sender);
            let sender2 = receiver.try_reset().unwrap();

            start_threads!(
                {
                    drop(sender2);
                },
                {
                    drop(receiver);
                }
            );
        }

        #[test]
        #[cfg_attr(miri, ignore)] // `sleep` not supported.
        fn send_and_reset() {
            let (sender, mut receiver) = new_oneshot();
            let (value, _check) = DropTest::new();

            start_threads!(
                {
                    sender.try_send(value).unwrap();
                },
                {
                    // Give the sender a chance to be dropped.
                    thread::sleep(Duration::from_millis(10));
                    let sender2 = receiver.try_reset().unwrap();
                    drop(sender2);
                    drop(receiver);
                }
            );
        }

        #[test]
        fn send_and_reset_and_drop() {
            let (sender, mut receiver) = new_oneshot();
            let (value, _check) = DropTest::new();
            sender.try_send(value).unwrap();
            let sender2 = receiver.try_reset().unwrap();

            start_threads!(
                {
                    drop(sender2);
                },
                {
                    drop(receiver);
                }
            );
        }
    }
}

mod threaded {
    use heph_inbox::oneshot::{new_oneshot, RecvError};

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

    #[test]
    #[cfg_attr(miri, ignore)] // Doesn't finish.
    fn single_value_send_and_received() {
        let (sender, mut receiver) = new_oneshot();

        start_threads!(
            {
                sender.try_send(1).unwrap();
            },
            {
                expect_recv!(receiver, 1);
            }
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Doesn't finish.
    fn single_value_send_and_received_zero_sized_types() {
        let (sender, mut receiver) = new_oneshot();

        start_threads!(
            {
                sender.try_send(()).unwrap();
            },
            {
                expect_recv!(receiver, ());
            }
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Doesn't finish.
    fn receiver_no_sender() {
        let (sender, mut receiver) = new_oneshot::<usize>();

        start_threads!(
            {
                drop(sender);
            },
            {
                r#loop! {
                    match receiver.try_recv() {
                        Ok(..) => panic!("unexpected receive of value"),
                        Err(RecvError::NoValue) => {} // Try again.
                        Err(RecvError::Disconnected) => break,
                    }
                }
            }
        );
    }

    #[test]
    fn send_no_receiver() {
        let (sender, receiver) = new_oneshot();

        start_threads!(
            {
                // NOTE: since we're racing with the `drop` below we can't know
                // the result.
                let _ = sender.try_send(1);
            },
            {
                drop(receiver);
            }
        );
    }

    #[test]
    fn sender_is_connected() {
        let (sender, receiver) = new_oneshot::<()>();

        start_threads!(
            {
                r#loop! {
                    if !sender.is_connected() {
                        break;
                    }
                }
            },
            {
                drop(receiver);
            }
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Doesn't finish.
    fn receiver_is_connected() {
        let (sender, receiver) = new_oneshot::<()>();

        start_threads!(
            {
                drop(sender);
            },
            {
                r#loop! {
                    if !receiver.is_connected() {
                        break;
                    }
                }
            }
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Doesn't finish.
    fn sender_usage_after_try_recv() {
        let (sender, mut receiver) = new_oneshot();

        start_threads!(
            {
                sender.try_send(123).unwrap();
            },
            {
                r#loop! {
                    match receiver.try_recv() {
                        Ok((msg, sender2)) => {
                            assert_eq!(msg, 123);
                            assert!(sender2.is_connected());

                            sender2.try_send(456).unwrap();
                            let (msg, sender3) = receiver.try_recv().unwrap();
                            assert_eq!(msg, 456);

                            assert!(sender3.is_connected());
                            assert!(receiver.is_connected());
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
}
