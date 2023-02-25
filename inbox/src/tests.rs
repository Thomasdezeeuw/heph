//! Tests for the internal API.

use std::future::Future;
use std::mem::{size_of, size_of_val};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{self, Poll, Wake};

use crate::{
    has_status, new_small, receiver_pos, slot_status, Channel, Join, Receiver, SendValue, Sender,
    ALL_STATUSES_MASK, EMPTY, FILLED, MARK_EMPTIED, MARK_NEXT_POS, MARK_READING, READING,
    SMALL_CAP, TAKEN,
};

/// Number of times the waker was awoken.
///
/// See [`new_count_waker`] for usage.
#[derive(Debug)]
struct AwokenCount {
    inner: Arc<WakerInner>,
}

impl PartialEq<usize> for AwokenCount {
    fn eq(&self, other: &usize) -> bool {
        self.inner.count.load(Ordering::SeqCst) == *other
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
fn new_count_waker() -> (task::Waker, AwokenCount) {
    let inner = Arc::new(WakerInner {
        count: AtomicUsize::new(0),
    });
    (inner.clone().into(), AwokenCount { inner })
}

#[test]
fn size_assertions() {
    let channel = unsafe { Box::from_raw(Channel::<()>::new(1).as_ptr()) };
    #[cfg(target_os = "linux")]
    assert_eq!(size_of_val(&**channel), 120);
    #[cfg(not(target_os = "linux"))]
    assert_eq!(size_of_val(&**channel), 136);
    assert_eq!(size_of::<Sender<()>>(), 16);
    assert_eq!(size_of::<Receiver<()>>(), 16);
    assert_eq!(size_of::<SendValue<()>>(), 40);
    assert_eq!(size_of::<Join<()>>(), 32);
}

#[test]
fn assertions() {
    // Various assertions that must be true for the channel to work
    // correctly.

    // Status are different.
    assert_ne!(EMPTY, TAKEN);
    assert_ne!(EMPTY, FILLED);
    assert_ne!(EMPTY, READING);
    assert_ne!(TAKEN, FILLED);
    assert_ne!(TAKEN, READING);
    assert_ne!(FILLED, READING);

    // Slot status marking operations.
    // In `Sender::try_send`.
    assert_eq!(EMPTY | TAKEN, TAKEN);
    assert_eq!(TAKEN | TAKEN, TAKEN);
    assert_eq!(EMPTY | FILLED, FILLED);
    assert_eq!(TAKEN | FILLED, FILLED);
    // In `Receiver::try_recv`.
    assert_eq!(FILLED ^ MARK_READING, READING);
    assert_eq!(FILLED & !MARK_EMPTIED, EMPTY);
    assert_eq!(READING & !MARK_EMPTIED, EMPTY);

    // Changing `Receiver` position doesn't change status of slots.
    const ORIGINAL_STATUS: u64 = 0b1110010011100100;
    assert_eq!(
        (size_of::<usize>() * 8) - (ORIGINAL_STATUS.leading_zeros() as usize),
        2 * SMALL_CAP
    );
    let mut status: u64 = ORIGINAL_STATUS.wrapping_sub(MARK_NEXT_POS);
    status = status.wrapping_add(MARK_NEXT_POS);
    assert_eq!(status, ORIGINAL_STATUS);
    status = status.wrapping_add(MARK_NEXT_POS);
    assert_eq!(status & ALL_STATUSES_MASK, ORIGINAL_STATUS);
}

#[test]
fn test_slot_status() {
    let tests = &[
        (0b00, 0, EMPTY),
        (0b01, 0, TAKEN),
        (0b10, 0, READING),
        (0b11, 0, FILLED),
        // Slot 1.
        (0b0000, 1, EMPTY),
        (0b0100, 1, TAKEN),
        (0b1000, 1, READING),
        (0b1100, 1, FILLED),
        // Slot 2.
        (0b000000, 2, EMPTY),
        (0b010000, 2, TAKEN),
        (0b100000, 2, READING),
        (0b110000, 2, FILLED),
        // Slot 3.
        (0b00000000, 3, EMPTY),
        (0b01000000, 3, TAKEN),
        (0b10000000, 3, READING),
        (0b11000000, 3, FILLED),
    ];

    for (input, slot, want) in tests.into_iter().copied() {
        assert_eq!(
            slot_status(input, slot),
            want,
            "input: {:064b}, slot: {}",
            input,
            slot,
        );
    }
}

#[test]
fn test_has_status() {
    let tests = &[
        // Slot 0.
        (0b00, 0, EMPTY, true),
        (0b00, 0, TAKEN, false),
        (0b00, 0, FILLED, false),
        (0b01, 0, EMPTY, false),
        (0b01, 0, TAKEN, true),
        (0b01, 0, FILLED, false),
        (0b11, 0, EMPTY, false),
        (0b11, 0, TAKEN, false),
        (0b11, 0, FILLED, true),
        // Slot 1.
        (0b0000, 1, EMPTY, true),
        (0b0000, 1, TAKEN, false),
        (0b0000, 1, FILLED, false),
        (0b0100, 1, EMPTY, false),
        (0b0100, 1, TAKEN, true),
        (0b0100, 1, FILLED, false),
        (0b1100, 1, EMPTY, false),
        (0b1100, 1, TAKEN, false),
        (0b1100, 1, FILLED, true),
        // Slot 1 filled, check slot 0.
        (0b0100, 0, EMPTY, true),
        (0b0100, 0, TAKEN, false),
        (0b0100, 0, FILLED, false),
        (0b1100, 0, EMPTY, true),
        (0b1100, 0, TAKEN, false),
        (0b1100, 0, FILLED, false),
    ];

    for (input, slot, expected, want) in tests.into_iter().copied() {
        assert_eq!(
            has_status(input, slot, expected),
            want,
            "input: {:064b}, slot: {}, expected: {:02b}",
            input,
            slot,
            expected
        );
    }
}

#[test]
fn test_receiver_pos() {
    #[rustfmt::skip]
    let tests = &[
        (0b0000000000000000000000000000000000000000000000000000000000000000, 0),
        (0b0000010000000000000000000000000000000000000000000000000000000000, 1),
        (0b0000100000000000000000000000000000000000000000000000000000000000, 2),
        (0b0000110000000000000000000000000000000000000000000000000000000000, 3),
        (0b0001000000000000000000000000000000000000000000000000000000000000, 4),
        (0b0001010000000000000000000000000000000000000000000000000000000000, 5),
        (0b0001100000000000000000000000000000000000000000000000000000000000, 6),
        (0b0001110000000000000000000000000000000000000000000000000000000000, 7),
        // Additional bits are ignored.
        (0b0000000000000000000000000000000000000000000000000000000000000000, 0),
        (0b1000010000000000000000000000000000000000000000000000000000000000, 1),
        (0b1000100000000000000000000000000000000000000000000000000000000000, 2),
        (0b0100110000000000000000000000000000000000000000000000000000000000, 3),
        (0b0011000000000000000000000000000000000000000000000000000000000000, 4),
        (0b0101010000000000000000000000000000000000000000000000000000000000, 5),
        (0b1001100000000000000000000000000000000000000000000000000000000000, 6),
        (0b1001110000000000000000000000000000000000000000000000000000000000, 7),
    ];

    for (input, want) in tests.into_iter().copied() {
        assert_eq!(
            receiver_pos(input, SMALL_CAP),
            want,
            "input: {:064b}",
            input
        );
    }
}

fn test_channel() -> Box<Channel<usize>> {
    unsafe { Box::from_raw(Channel::new(SMALL_CAP).as_ptr()) }
}

#[test]
fn channel_next_sender_waker_none() {
    let channel = test_channel();
    channel.wake_next_sender();
}

#[test]
fn channel_next_sender_waker_single_waker() {
    let channel = test_channel();
    let (waker, count) = new_count_waker();

    channel.sender_wakers.lock().unwrap().push(waker);

    channel.wake_next_sender();
    assert_eq!(count, 1);
    assert!(channel.sender_wakers.lock().unwrap().is_empty());
}

#[test]
fn channel_next_sender_waker_two_wakers() {
    let channel = test_channel();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    {
        let mut sender_wakers = channel.sender_wakers.lock().unwrap();
        sender_wakers.push(waker1);
        sender_wakers.push(waker2);
    }

    channel.wake_next_sender();
    assert_eq!(count1, 1);
    assert_eq!(count2, 0);
    channel.wake_next_sender();
    assert_eq!(count1, 1);
    assert_eq!(count2, 1);
    assert!(channel.sender_wakers.lock().unwrap().is_empty());
}

#[test]
fn channel_next_sender_waker_three_wakers() {
    let channel = test_channel();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    {
        let mut sender_wakers = channel.sender_wakers.lock().unwrap();
        sender_wakers.push(waker1);
        sender_wakers.push(waker2);
        sender_wakers.push(waker3);
    }

    channel.wake_next_sender();
    assert_eq!(count1, 1);
    assert_eq!(count2, 0);
    assert_eq!(count3, 0);
    channel.wake_next_sender();
    assert_eq!(count1, 1);
    assert_eq!(count2, 0); // NOTE: waking order is not guaranteed.
    assert_eq!(count3, 1);
    channel.wake_next_sender();
    assert_eq!(count1, 1);
    assert_eq!(count2, 1);
    assert_eq!(count3, 1);
    assert!(channel.sender_wakers.lock().unwrap().is_empty());
}

#[test]
fn send_value_removes_waker_from_list_on_drop() {
    let (sender, mut receiver) = new_small::<usize>();

    for _ in 0..sender.capacity() {
        sender.try_send(123).unwrap();
    }

    let (waker, count) = new_count_waker();
    let mut ctx = task::Context::from_waker(&waker);

    let mut future = Box::pin(sender.send(10));
    assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

    // Dropping the `SendValue` future should remove the waker from the list.
    drop(future);
    assert!(receiver.channel().sender_wakers.lock().unwrap().is_empty());

    for _ in 0..receiver.capacity() {
        assert_eq!(receiver.try_recv().unwrap(), 123);
    }
    drop(receiver);

    assert_eq!(count, 0);
}

#[test]
fn send_value_removes_waker_from_list_on_drop_polled_with_different_wakers() {
    let (sender, mut receiver) = new_small::<usize>();

    for _ in 0..sender.capacity() {
        sender.try_send(123).unwrap();
    }

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let mut ctx1 = task::Context::from_waker(&waker1);
    let mut ctx2 = task::Context::from_waker(&waker2);

    let mut future = Box::pin(sender.send(10));
    assert_eq!(future.as_mut().poll(&mut ctx1), Poll::Pending);
    assert_eq!(future.as_mut().poll(&mut ctx2), Poll::Pending);

    // Dropping the `SendValue` future should remove the waker from the list.
    drop(future);
    assert!(receiver.channel().sender_wakers.lock().unwrap().is_empty());

    for _ in 0..receiver.capacity() {
        assert_eq!(receiver.try_recv().unwrap(), 123);
    }
    drop(receiver);

    assert_eq!(count1, 0);
    assert_eq!(count2, 0);
}
