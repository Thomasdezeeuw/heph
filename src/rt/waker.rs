//! Module containing the `task::Waker` implementation.

use std::mem::size_of;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task;

use crossbeam_channel::Sender;
use log::{error, trace};

use crate::rt::thread_waker::ThreadWaker;
use crate::rt::ProcessId;

/// Maximum number of threads currently supported by this `Waker`
/// implementation.
pub const MAX_THREADS: usize = 128;

/// An id for a waker.
///
/// Returned by `init` and used in `new` to create a new `Waker`.
//
// This serves as index into `THREAD_WAKERS`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub(crate) struct WakerId(u8);

/// Initialise a new waker.
///
/// This returns a `WakerId` which can be used to create a new `Waker` using
/// `new`.
pub(crate) fn init(waker: mio::Waker, notifications: Sender<ProcessId>) -> WakerId {
    /// Each worker thread that uses a `Waker` implementation needs an unique
    /// `WakerId`, which serves as index to `THREAD_WAKERS`, this static
    /// determines that.
    static THREAD_IDS: AtomicU8 = AtomicU8::new(0);

    let thread_id = THREAD_IDS.fetch_add(1, Ordering::SeqCst);
    if thread_id as usize >= MAX_THREADS {
        panic!("Created too many Heph worker threads");
    }

    // Safety: this is safe because we are the only thread that has write access
    // to the given index. See documentation of `THREAD_WAKERS` for more.
    unsafe {
        THREAD_WAKERS[thread_id as usize] = Some(Waker {
            notifications,
            thread_waker: ThreadWaker::new(waker),
        });
    }
    WakerId(thread_id)
}

/// Create a new `task::Waker`.
///
/// `init` must be called before calling this function to get a `WakerId`.
pub(crate) fn new(waker_id: WakerId, pid: ProcessId) -> task::Waker {
    let data = WakerData::new(waker_id, pid).into_raw_data();
    let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
    // Safety: we follow the contract on `RawWaker`.
    unsafe { task::Waker::from_raw(raw_waker) }
}

/// Let the `Waker` know the worker thread is currently `polling` (or not).
///
/// This is used by the `Waker` implementation to wake the worker thread up from
/// polling.
pub(crate) fn mark_polling(waker_id: WakerId, polling: bool) {
    get(waker_id).thread_waker.mark_polling(polling);
}

/// Each worker thread of the `Runtime` has a unique `WakeId` which is used as
/// index into this array.
///
/// # Safety
///
/// Only `init` may write to this array. After the initial write, no more writes
/// are allowed and the array element is read only. To get a waker use the
/// `get_waker` function.
///
/// Following the rules above means that there are no data races. The array can
/// only be indexed by `WakerId`, which is only created by `init`, which ensures
/// the waker is setup before returning the `WakerId`. This ensures that only a
/// single write happens to each element of the array. And because after the
/// initial write each element is read only there are no further data races
/// possible.
static mut THREAD_WAKERS: [Option<Waker>; MAX_THREADS] = [NO_WAKER; MAX_THREADS];
// NOTE: this is only here because `NO_WAKER` is not `Copy`, thus
// `[None; MAX_THREADS]` doesn't work, but explicitly using a `const` does.
const NO_WAKER: Option<Waker> = None;

/// Get waker data for `waker_id`
fn get(waker_id: WakerId) -> &'static Waker {
    // Safety: `WakerId` is only created by `init`, which ensures its valid.
    // Furthermore `init` ensures that `THREAD_WAKER[waker_id]` is initialised
    // and is read-only after that. See `THREAD_WAKERS` documentation for more.
    unsafe {
        THREAD_WAKERS[waker_id.0 as usize]
            .as_ref()
            .expect("tried to get a waker for a thread that isn't initialised")
    }
}

/// Returns the `ThreadWaker` for `waker_id`.
pub(crate) fn get_thread_waker(waker_id: WakerId) -> &'static ThreadWaker {
    &get(waker_id).thread_waker
}

/// Waker mechanism.
#[derive(Debug)]
struct Waker {
    notifications: Sender<ProcessId>,
    thread_waker: ThreadWaker,
}

impl Waker {
    /// Wake up the process with `pid`.
    fn wake(&self, pid: ProcessId) {
        trace!("waking: pid={}", pid);
        if let Err(err) = self.notifications.try_send(pid) {
            error!("unable to send wake up notification: {}", err);
            return;
        }

        self.wake_thread()
    }

    /// Wake up the thread, without waking a specific process.
    fn wake_thread(&self) {
        if let Err(err) = self.thread_waker.wake() {
            error!("unable to wake up worker thread: {}", err);
        }
    }
}

/// Waker data passed to `LocalWaker` and `Waker` implementations.
///
/// # Layout
///
/// The 32 least significant bits (right-most) make up the process id
/// (`ProcessId`), the next 8 bits are the waker id (`WakerId`). The 8 most
/// significant bits are currently unused.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakerData(usize);

const THREAD_BITS: usize = 8;
const THREAD_SHIFT: usize = (size_of::<*const ()>() * 8) - THREAD_BITS;
const THREAD_MASK: usize = ((1 << THREAD_BITS) - 1) << THREAD_SHIFT;

impl WakerData {
    /// Create new `WakerData`.
    fn new(thread_id: WakerId, pid: ProcessId) -> WakerData {
        debug_assert!(pid.0 < (1 << THREAD_SHIFT), "pid too large");
        WakerData((thread_id.0 as usize) << THREAD_SHIFT | pid.0)
    }

    /// Get the thread id of from the waker data.
    const fn waker_id(self) -> WakerId {
        // Safety: `WakerId` is u8, so no truncating.
        #[allow(clippy::cast_possible_truncation)]
        WakerId((self.0 >> THREAD_SHIFT) as u8)
    }

    /// Get the process id from the waker data.
    const fn pid(self) -> ProcessId {
        // Safety: checked pid in `WakerData::new`, so no truncation.
        #[allow(clippy::cast_possible_truncation)]
        ProcessId(self.0 & !THREAD_MASK)
    }

    /// Convert raw data from `RawWaker` into `WakerData`.
    ///
    /// # Safety
    ///
    /// This doesn't check if the provided `data` is valid, the caller is
    /// responsible for this.
    const unsafe fn from_raw_data(data: *const ()) -> WakerData {
        WakerData(data as usize)
    }

    /// Convert `WakerData` into raw data for `RawWaker`.
    const fn into_raw_data(self) -> *const () {
        self.0 as *const ()
    }
}

/// Virtual table used by the `Waker` implementation.
static WAKER_VTABLE: task::RawWakerVTable =
    task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

fn assert_copy<T: Copy>() {}

unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
    assert_copy::<WakerData>();
    // Since the data is `Copy`, so we just copy it.
    task::RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    // This is safe because we received the data from the `RawWaker`, which
    // doesn't modify the data.
    let data = WakerData::from_raw_data(data);
    get(data.waker_id()).wake(data.pid())
}

unsafe fn wake_by_ref(data: *const ()) {
    assert_copy::<WakerData>();
    // Since we `WakerData` is `Copy` `wake` doesn't actually consume any data,
    // so we can just call it.
    wake(data)
}

unsafe fn drop_wake_data(data: *const ()) {
    assert_copy::<WakerData>();
    // Since the data is `Copy` we don't have to anything.
    #[allow(clippy::drop_copy)]
    drop(data)
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::thread;
    use std::time::Duration;

    use mio::{Events, Poll, Token, Waker};

    use crate::rt::waker::{self, WakerData};
    use crate::rt::ProcessId;

    use super::{MAX_THREADS, THREAD_BITS, THREAD_MASK};

    const WAKER: Token = Token(0);
    const PID1: ProcessId = ProcessId(0);
    const PID2: ProcessId = ProcessId(1);

    #[test]
    fn assert_waker_data_size() {
        assert_eq!(size_of::<*const ()>(), size_of::<WakerData>());
    }

    #[test]
    fn thread_bits_large_enough() {
        assert!(
            2usize.pow(THREAD_BITS as u32) >= MAX_THREADS,
            "Not enough bits for MAX_THREADS"
        );
    }

    #[test]
    fn thread_mask() {
        assert!(
            (usize::MAX & !THREAD_MASK).leading_zeros() as usize == THREAD_BITS,
            "Incorrect THREAD_MASK"
        );
    }

    #[test]
    fn waker() {
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(8);

        // Initialise the waker.
        let waker = Waker::new(poll.registry(), WAKER).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = waker::init(waker, wake_sender);

        // Create a new waker.
        let waker = waker::new(waker_id, PID1);

        // Should receive an event for the `Waker` if marked as polling.
        waker::mark_polling(waker_id, true);
        waker.wake();
        poll.poll(&mut events, Some(Duration::from_secs(1)))
            .unwrap();
        expect_one_waker_event(&mut events);
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
        waker::mark_polling(waker_id, false);

        let pid2 = ProcessId(usize::MAX & !THREAD_MASK);
        let waker = waker::new(waker_id, pid2);
        waker::mark_polling(waker_id, true);
        waker.wake();
        poll.poll(&mut events, Some(Duration::from_secs(1)))
            .unwrap();
        // Should receive an event for the Waker.
        expect_one_waker_event(&mut events);
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(pid2));
    }

    #[test]
    fn waker_not_polling() {
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(8);

        // Initialise the waker.
        let waker = Waker::new(poll.registry(), WAKER).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = waker::init(waker, wake_sender);

        // Create a new waker.
        let waker = waker::new(waker_id, PID1);

        // Since the waker is marked as not polling we shouldn't get an event.
        waker::mark_polling(waker_id, false);
        waker.wake();
        poll.poll(&mut events, Some(Duration::from_millis(100)))
            .unwrap();
        assert!(events.is_empty());
        // But the pid should still be delivered
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
    }

    #[test]
    fn waker_single_mio_waker_call() {
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(8);

        let waker = Waker::new(poll.registry(), WAKER).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = waker::init(waker, wake_sender);

        let waker = waker::new(waker_id, PID1);
        let waker2 = waker::new(waker_id, PID2);

        // Should receive an event for the `Waker` if marked as polling.
        waker::mark_polling(waker_id, true);
        waker.wake_by_ref();
        waker.wake(); // More calls shouldn't create more `WAKER` events.
        waker2.wake_by_ref();
        waker2.wake();
        poll.poll(&mut events, Some(Duration::from_secs(1)))
            .unwrap();
        expect_one_waker_event(&mut events);
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
        assert_eq!(wake_receiver.try_recv(), Ok(PID2));
        assert_eq!(wake_receiver.try_recv(), Ok(PID2));
    }

    #[test]
    fn waker_different_thread() {
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(8);

        // Initialise the waker.
        let waker = Waker::new(poll.registry(), WAKER).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = waker::init(waker, wake_sender);

        waker::mark_polling(waker_id, true);
        // Create a new waker.
        let waker = waker::new(waker_id, PID1);
        let handle = thread::spawn(move || {
            waker.wake();
        });

        handle.join().unwrap();
        poll.poll(&mut events, Some(Duration::from_secs(1)))
            .unwrap();
        // Should receive an event for the Waker.
        expect_one_waker_event(&mut events);
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
    }

    fn expect_one_waker_event(events: &mut Events) {
        assert!(!events.is_empty());
        let mut iter = events.iter();
        let event = iter.next().unwrap();
        assert_eq!(event.token(), WAKER);
        assert!(event.is_readable());
        assert!(iter.next().is_none(), "unexpected event");
    }
}
