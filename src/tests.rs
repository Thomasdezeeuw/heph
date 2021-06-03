//! Tests for the internal API.

use std::future::Future;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::AtomicPtr;
use std::task::{self, Poll};

use futures_test::task::new_count_waker;
use parking_lot::Mutex;

use crate::{
    has_status, new_small, receiver_pos, slot_status, Channel, SendValue, WakerList,
    ALL_STATUSES_MASK, EMPTY, FILLED, MARK_EMPTIED, MARK_NEXT_POS, MARK_READING, POS_BITS, READING,
    SMALL_CAP, TAKEN,
};

#[test]
fn size_assertions() {
    assert_eq!(size_of::<Channel<()>>(), 88);
    assert_eq!(size_of::<SendValue<()>>(), 56);
}

#[test]
fn assertions() {
    // Various assertions that must be true for the channel to work
    // correctly.

    // Enough bits for the statuses of the slots.
    assert_eq!(2_usize.pow(POS_BITS as u32), SMALL_CAP);

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
    const ORIGINAL_STATUS: usize = 0b1110010011100100;
    assert_eq!(
        (size_of::<usize>() * 8) - (ORIGINAL_STATUS.leading_zeros() as usize),
        2 * SMALL_CAP
    );
    let mut status: usize = ORIGINAL_STATUS.wrapping_sub(MARK_NEXT_POS);
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
    let tests = &[
        (0b000_1110010011100100, 0),
        (0b001_1110010011100100, 1),
        (0b010_1110010011100100, 2),
        (0b011_1110010011100100, 3),
        (0b100_1110010011100100, 4),
        (0b101_1110010011100100, 5),
        (0b110_1110010011100100, 6),
        (0b111_1110010011100100, 7),
        // Additional bits are ignored.
        (0b1_000_1110010011100100, 0),
        (0b1_001_1110010011100100, 1),
        (0b1_010_1110010011100100, 2),
        (0b1_011_1110010011100100, 3),
        (0b1_100_1110010011100100, 4),
        (0b1_101_1110010011100100, 5),
        (0b1_110_1110010011100100, 6),
        (0b1_111_1110010011100100, 7),
    ];

    for (input, want) in tests.into_iter().copied() {
        assert_eq!(receiver_pos(input), want, "input: {:064b}", input);
    }
}

#[test]
fn channel_next_sender_waker_none() {
    let channel = Channel::<usize>::new();
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_single_waker() {
    let channel = Channel::<usize>::new();

    let (waker, count) = new_count_waker();

    let mut wakers = [WakerList {
        waker: Mutex::new(Some(waker)),
        next: AtomicPtr::new(ptr::null_mut()),
    }];

    unsafe { channel.add_sender_waker(&mut wakers[0]) };

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_two_wakers() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
    }

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_three_wakers() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert_eq!(count3.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert_eq!(count3.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_with_none_waker_0() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(None),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_with_none_waker_1() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(None),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_next_sender_waker_with_none_waker_2() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(None),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 0);
    channel.next_sender_waker().unwrap().wake();
    assert_eq!(count1.get(), 1);
    assert_eq!(count2.get(), 1);
    assert!(channel.next_sender_waker().is_none());
}

#[test]
fn channel_remove_sender_waker_single_waker() {
    let channel = Channel::<usize>::new();

    let (waker, count) = new_count_waker();

    let mut wakers = [WakerList {
        waker: Mutex::new(Some(waker)),
        next: AtomicPtr::new(ptr::null_mut()),
    }];

    unsafe { channel.add_sender_waker(&mut wakers[0]) };
    channel.remove_sender_waker(&wakers[0]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count.get(), 0);
}

#[test]
fn channel_remove_sender_waker_two_wakers_same_order() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
    }

    channel.remove_sender_waker(&mut wakers[0]);
    channel.remove_sender_waker(&mut wakers[1]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
}

#[test]
fn channel_remove_sender_waker_two_wakers_reverse_order() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
    }

    channel.remove_sender_waker(&mut wakers[1]);
    channel.remove_sender_waker(&mut wakers[0]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_0() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[0]);
    channel.remove_sender_waker(&mut wakers[1]);
    channel.remove_sender_waker(&mut wakers[2]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_1() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[0]);
    channel.remove_sender_waker(&mut wakers[2]);
    channel.remove_sender_waker(&mut wakers[1]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_2() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[1]);
    channel.remove_sender_waker(&mut wakers[0]);
    channel.remove_sender_waker(&mut wakers[2]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_3() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[1]);
    channel.remove_sender_waker(&mut wakers[2]);
    channel.remove_sender_waker(&mut wakers[0]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_4() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[2]);
    channel.remove_sender_waker(&mut wakers[0]);
    channel.remove_sender_waker(&mut wakers[1]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_three_wakers_order_5() {
    let channel = Channel::<usize>::new();

    let (waker1, count1) = new_count_waker();
    let (waker2, count2) = new_count_waker();
    let (waker3, count3) = new_count_waker();

    let mut wakers = [
        WakerList {
            waker: Mutex::new(Some(waker1)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker2)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
        WakerList {
            waker: Mutex::new(Some(waker3)),
            next: AtomicPtr::new(ptr::null_mut()),
        },
    ];

    unsafe {
        channel.add_sender_waker(&mut wakers[0]);
        channel.add_sender_waker(&mut wakers[1]);
        channel.add_sender_waker(&mut wakers[2]);
    }

    channel.remove_sender_waker(&mut wakers[2]);
    channel.remove_sender_waker(&mut wakers[1]);
    channel.remove_sender_waker(&mut wakers[0]);

    assert!(channel.next_sender_waker().is_none());
    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
    assert_eq!(count3.get(), 0);
}

#[test]
fn channel_remove_sender_waker_not_in_list() {
    let channel = Channel::<usize>::new();
    let waker = WakerList {
        waker: Mutex::new(None),
        next: AtomicPtr::new(ptr::null_mut()),
    };
    channel.remove_sender_waker(&waker);
}

#[test]
fn channel_remove_sender_waker_null_pointer() {
    let channel = Channel::<usize>::new();
    channel.remove_sender_waker(ptr::null());
}

#[test]
fn send_value_removes_waker_from_list_on_drop() {
    let (sender, mut receiver) = new_small::<usize>();

    for _ in 0..SMALL_CAP {
        sender.try_send(123).unwrap();
    }

    let (waker, count) = new_count_waker();
    let mut ctx = task::Context::from_waker(&waker);

    let mut future = Box::pin(sender.send(10));
    assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

    // Dropping the `SendValue` future should remove the waker from the list.
    drop(future);
    assert!(receiver.channel().next_sender_waker().is_none());

    for _ in 0..SMALL_CAP {
        assert_eq!(receiver.try_recv().unwrap(), 123);
    }
    drop(receiver);

    assert_eq!(count.get(), 0);
}

#[test]
fn send_value_removes_waker_from_list_on_drop_polled_with_different_wakers() {
    let (sender, mut receiver) = new_small::<usize>();

    for _ in 0..SMALL_CAP {
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
    assert!(receiver.channel().next_sender_waker().is_none());

    for _ in 0..SMALL_CAP {
        assert_eq!(receiver.try_recv().unwrap(), 123);
    }
    drop(receiver);

    assert_eq!(count1.get(), 0);
    assert_eq!(count2.get(), 0);
}
