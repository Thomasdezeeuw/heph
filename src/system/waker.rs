//! Module containing the `task::Waker` implementation.

use std::mem;
use std::sync::atomic::{AtomicU16, Ordering};
use std::task::{RawWaker, RawWakerVTable, Waker};

use crossbeam_channel::Sender;
use gaea::os::Awakener;
use log::error;

use crate::system::ProcessId;

/// Maximum number of threads currently supported by this `Waker`
/// implementation.
pub const MAX_THREADS: usize = 64;

/// Each worker thread that uses a `Waker` implementation needs an unique
/// `WakerId`, which serves as index into `THREAD_WAKERS`, this variable
/// determines that.
static THREAD_IDS: AtomicU16 = AtomicU16::new(0);

/// Each worker thread of the `ActorSystem` has a unique `WakeId` which is used
/// as index into the array.
///
/// # Safety
///
/// Only `init_waker` may write to this array. After the initial write, no more
/// writes are allowed and the array element is read only.
///
/// Following the rules above means that there are no data races. The array can
/// only be index by `WakerId`, which is only created by `init_waker`, which
/// ensures the waker is setup before returning the `WakerId`. This ensure that
/// only a single write happens to each element of the array. And because after
/// the initial write each element is read only there are no further data races
/// possible.
static mut THREAD_WAKERS: [Option<ThreadWaker>; MAX_THREADS] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

/// A `Waker` implementation.
struct ThreadWaker {
    notifications: Sender<ProcessId>,
    awakener: Awakener,
}

impl ThreadWaker {
    /// Wake up the process with `pid`.
    fn wake(&self, pid: ProcessId) {
        if let Err(err) = self.notifications.try_send(pid) {
            error!("unable to send wake up notification: {}", err);
            return;
        }

        if let Err(err) = self.awakener.wake() {
            error!("unable to wake up worker thread: {}", err);
        }
    }
}

/// An id for a waker.
///
/// Returned by `init_waker` and used in `new_waker` to create a new `Waker`.
///
/// This served as index into `THREAD_WAKERS`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct WakerId(u16);

/// Initialise a new waker.
///
/// This returns a `WakerId` which can be used to create a new `Waker` using
/// `new_waker`.
pub fn init_waker(awakener: Awakener, notifications: Sender<ProcessId>) -> WakerId {
    let thread_id = THREAD_IDS.fetch_add(1, Ordering::AcqRel);
    if thread_id as usize >= MAX_THREADS {
        panic!("Created too many Heph worker threads");
    }

    // This is safe because we are the only thread that has write access to the
    // given index. See documentation of `THREAD_WAKERS` for more.
    unsafe {
        THREAD_WAKERS[thread_id as usize] = Some(ThreadWaker {
            awakener,
            notifications,
        });
    }
    WakerId(thread_id)
}

/// Create a new `Waker`.
///
/// `init_waker` must be called before calling this function to get a `WakerId`.
pub fn new_waker(waker_id: WakerId, pid: ProcessId) -> Waker {
    let data = WakerData::new(waker_id, pid).into_raw_data();
    let raw_waker = RawWaker::new(data, WAKER_VTABLE);
    unsafe { Waker::from_raw(raw_waker) }
}

/// Waker data passed to `LocalWaker` and `Waker` implementations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakerData(usize);

const WAKER_DATA_BITS: usize = mem::size_of::<WakerData>() * 8;
const THREAD_ID_BITS: usize = mem::size_of::<WakerId>() * 8;

impl WakerData {
    /// Create new `WakerData`.
    fn new(thread_id: WakerId, pid: ProcessId) -> WakerData {
        WakerData((thread_id.0 as usize) << (WAKER_DATA_BITS - THREAD_ID_BITS) | pid.0 as usize)
    }

    /// Get the thread id of from the waker data.
    fn waker_id(self) -> WakerId {
        WakerId((self.0 >> (WAKER_DATA_BITS - THREAD_ID_BITS)) as u16)
    }

    /// Get the process id from the waker data.
    fn pid(self) -> ProcessId {
        ProcessId(((self.0 << THREAD_ID_BITS) >> THREAD_ID_BITS) as u32)
    }

    /// Convert raw data from `RawWaker` into `WakerData`.
    ///
    /// # Safety
    ///
    /// This doesn't check if the provided `data` is valid, the caller is
    /// responsible for this.
    unsafe fn from_raw_data(data: *const ()) -> WakerData {
        WakerData(data as usize)
    }

    /// Convert `WakerData` into raw data for `RawWaker`.
    fn into_raw_data(self) -> *const () {
        self.0 as *const ()
    }
}

/// Virtual table used by the `Waker` implementation.
static WAKER_VTABLE: &RawWakerVTable =
    &RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

fn assert_copy<T: Copy>() {}

unsafe fn clone_wake_data(data: *const ()) -> RawWaker {
    assert_copy::<WakerData>();
    // Since the data is `Copy`, so we just copy it and return use same vtable.
    RawWaker::new(data, WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    // This is safe because we received the data from the `RawWaker`, which
    // doesn't modify the data.
    let data = WakerData::from_raw_data(data);
    let waker_id = data.waker_id();
    let pid = data.pid();

    // This is safe as the only way the `waker_id` is created is by
    // `init_waker`, which ensure that the particular index is set. See
    // `THREAD_WAKERS` documentation for more.
    let thread_waker = THREAD_WAKERS[waker_id.0 as usize]
        .as_ref()
        .expect("tried to wake a thread that isn't initialised");
    thread_waker.wake(pid)
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
    use std::{io, thread};

    use gaea::os::{Awakener, OsQueue};
    use gaea::{event, poll, Event, Ready};

    use crate::system::ProcessId;
    use crate::system::waker::{init_waker, new_waker, WakerData};

    const AWAKENER_ID: event::Id = event::Id(0);
    const PID1: ProcessId = ProcessId(0);

    #[test]
    fn assert_waker_data_size() {
        assert_eq!(size_of::<*const ()>(), size_of::<WakerData>());
    }

    #[test]
    fn waker() {
        let mut os_queue = OsQueue::new().unwrap();
        let mut events = Vec::new();

        // Initialise the waker.
        let awakener = Awakener::new(&mut os_queue, AWAKENER_ID).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = init_waker(awakener, wake_sender);

        // Create a new waker.
        let waker = new_waker(waker_id, PID1);
        waker.wake();

        poll::<_, io::Error>(&mut [&mut os_queue], &mut events, None).unwrap();
        // Should receive an event for the Awakener.
        assert_eq!(events.len(), 1);
        assert_eq!(events.pop(), Some(Event::new(AWAKENER_ID, Ready::READABLE)));
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));

        let pid2 = ProcessId(u32::max_value());
        let waker = new_waker(waker_id, pid2);
        waker.wake();

        poll::<_, io::Error>(&mut [&mut os_queue], &mut events, None).unwrap();
        // Should receive an event for the Awakener.
        assert_eq!(events.len(), 1);
        assert_eq!(events.pop(), Some(Event::new(AWAKENER_ID, Ready::READABLE)));
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(pid2));
    }

    #[test]
    fn waker_different_thread() {
        let mut os_queue = OsQueue::new().unwrap();
        let mut events = Vec::new();

        // Initialise the waker.
        let awakener = Awakener::new(&mut os_queue, AWAKENER_ID).unwrap();
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let waker_id = init_waker(awakener, wake_sender);

        // Create a new waker.
        let waker = new_waker(waker_id, PID1);
        let handle = thread::spawn(move || {
            waker.wake();
        });

        handle.join().unwrap();
        poll::<_, io::Error>(&mut [&mut os_queue], &mut events, None).unwrap();
        // Should receive an event for the Awakener.
        assert_eq!(events.len(), 1);
        assert_eq!(events.pop(), Some(Event::new(AWAKENER_ID, Ready::READABLE)));
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
    }
}
