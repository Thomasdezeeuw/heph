//! Module containing the `task::Waker` implementation for thread-safe actors.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Weak;
use std::task;

use crate::rt::shared::RuntimeInternals;
use crate::rt::ProcessId;

/// Maximum number of runtimes supported.
pub const MAX_RUNTIMES: usize = 1 << MAX_RUNTIMES_BITS;
#[cfg(not(any(test, feature = "test")))]
pub const MAX_RUNTIMES_BITS: usize = 0; // 1.
#[cfg(any(test, feature = "test"))]
pub const MAX_RUNTIMES_BITS: usize = 4; // 16.

/// An id for a waker.
///
/// Returned by [`init`] and used in [`new`] to create a new [`task::Waker`].
//
// This serves as index into `WAKERS`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub(crate) struct WakerId(u8);

/// Each coordinator has a unique [`WakerId`] which is used as index into this
/// array.
///
/// # Safety
///
/// Only [`init`] may write to this array. After the initial write, no more
/// writes are allowed and the array element is read only. To get a
/// [`task::Waker`] use the [`new`] function.
///
/// Following the rules above means that there are no data races. The array can
/// only be indexed by [`WakerId`], which is only created by [`init`], which
/// ensures the waker is setup before returning the [`WakerId`]. This ensures
/// that only a single write happens to each element of the array. And because
/// after the initial write each element is read only there are no further data
/// races possible.
static mut RUNTIMES: [Option<Weak<RuntimeInternals>>; MAX_RUNTIMES] = [NO_RUNTIME; MAX_RUNTIMES];
// NOTE: this is only here because `NO_WAKER` is not `Copy`, thus
// `[None; MAX_THREADS]` doesn't work, but explicitly using a `const` does.
const NO_RUNTIME: Option<Weak<RuntimeInternals>> = None;

/// Initialise a new waker.
///
/// This returns a [`WakerId`] which can be used to create a new [`task::Waker`]
/// using [`new`].
pub(crate) fn init(internals: Weak<RuntimeInternals>) -> WakerId {
    /// Static used to determine unique indices into `RUNTIMES`.
    static IDS: AtomicU8 = AtomicU8::new(0);

    let id = IDS.fetch_add(1, Ordering::SeqCst);
    if id as usize >= MAX_RUNTIMES {
        panic!(
            "Created too many Heph `Runtime`s, maximum of {}",
            MAX_RUNTIMES
        );
    }

    // Safety: this is safe because we are the only thread that has write access
    // to the given index. See documentation of `WAKERS` for more.
    unsafe { RUNTIMES[id as usize] = Some(internals) }
    WakerId(id)
}

/// Create a new [`task::Waker`].
///
/// [`init`] must be called before calling this function to get a [`WakerId`].
pub(crate) fn new(waker_id: WakerId, pid: ProcessId) -> task::Waker {
    let data = WakerData::new(waker_id, pid).into_raw_data();
    let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
    // Safety: we follow the contract on `RawWaker`.
    unsafe { task::Waker::from_raw(raw_waker) }
}

/// Get the internals for `waker_id`.
fn get(waker_id: WakerId) -> &'static Weak<RuntimeInternals> {
    // Safety: `WakerId` is only created by `init`, which ensures its valid.
    // Furthermore `init` ensures that `RUNTIMES[waker_id]` is initialised and
    // is read-only after that. See `RUNTIMES` documentation for more.
    unsafe {
        RUNTIMES[waker_id.0 as usize]
            .as_ref()
            .expect("tried to get a waker for a thread that isn't initialised")
    }
}

/// Waker data passed to the [`task::Waker`] implementation.
///
/// # Layout
///
/// The [`MAX_RUNTIMES_BITS`] least significant bits are the [`WakerId`]. The
/// remaining bits are the [`ProcessId`], from which at least
/// `MAX_RUNTIMES_BITS` most significant bits are not used.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakerData(usize);

const WAKER_ID_MASK: usize = (1 << MAX_RUNTIMES_BITS) - 1;

impl WakerData {
    /// Create new `WakerData`.
    fn new(waker_id: WakerId, pid: ProcessId) -> WakerData {
        let data =
            WakerData((pid.0 << MAX_RUNTIMES_BITS) | ((waker_id.0 as usize) & WAKER_ID_MASK));
        assert!(data.pid() == pid, "`ProcessId` too large for `WakerData`");
        data
    }

    /// Get the waker id.
    const fn waker_id(self) -> WakerId {
        // Safety: we know we won't truncate the waker id as it's an u8.
        #[allow(clippy::cast_possible_truncation)]
        WakerId((self.0 & WAKER_ID_MASK) as u8)
    }

    /// Get the process id.
    const fn pid(self) -> ProcessId {
        // Safety: we know we won't truncate the pid, we check in
        // `WakerData::new`.
        #[allow(clippy::cast_possible_truncation)]
        ProcessId(self.0 >> MAX_RUNTIMES_BITS)
    }

    /// Convert raw data from [`task::RawWaker`] into [`WakerData`].
    ///
    /// # Safety
    ///
    /// This doesn't check if the provided `data` is valid, the caller is
    /// responsible for this.
    const unsafe fn from_raw_data(data: *const ()) -> WakerData {
        WakerData(data as usize)
    }

    /// Convert [`WakerData`] into raw data for [`task::RawWaker`].
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
    // Since the data is `Copy`, we just copy it.
    task::RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    // This is safe because we received the data from the `RawWaker`, which
    // doesn't modify the data.
    let data = WakerData::from_raw_data(data);
    if let Some(shared_internals) = get(data.waker_id()).upgrade() {
        shared_internals.mark_ready(data.pid());
        shared_internals.wake_workers(1);
    }
}

unsafe fn wake_by_ref(data: *const ()) {
    assert_copy::<WakerData>();
    // Since `WakerData` is `Copy` `wake` doesn't actually consume any data, so
    // we can just call it.
    wake(data)
}

unsafe fn drop_wake_data(_: *const ()) {
    assert_copy::<WakerData>();
    // Since the data is `Copy` we don't have to do anything.
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, Weak};
    use std::thread::{self, sleep};
    use std::time::Duration;

    use mio::Poll;

    use crate::rt::options::Priority;
    use crate::rt::process::{Process, ProcessData, ProcessId, ProcessResult};
    use crate::rt::shared::waker::{self, WakerData};
    use crate::rt::shared::{RuntimeInternals, Scheduler};
    use crate::rt::{RuntimeRef, Timers};
    use crate::test;

    const PID1: ProcessId = ProcessId(1);
    const PID2: ProcessId = ProcessId(2);

    #[test]
    fn assert_waker_data_size() {
        assert_eq!(size_of::<*const ()>(), size_of::<WakerData>());
    }

    struct TestProcess;

    impl Process for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }

        fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
            unimplemented!();
        }
    }

    #[test]
    fn waker() {
        let shared_internals = new_internals();

        let pid = add_process(&shared_internals.scheduler);
        assert!(shared_internals.scheduler.has_process());
        assert!(!shared_internals.scheduler.has_ready_process());

        // Create a new waker.
        let waker = waker::new(shared_internals.shared_id, pid);

        // Waking should move the process to the ready queue.
        waker.wake_by_ref();
        assert!(shared_internals.scheduler.has_process());
        assert!(shared_internals.scheduler.has_ready_process());
        let process = shared_internals.scheduler.remove().unwrap();
        assert_eq!(process.as_ref().id(), pid);

        // Waking a process that isn't in the scheduler should be fine.
        waker.wake();
        assert!(!shared_internals.scheduler.has_process());
        assert!(!shared_internals.scheduler.has_ready_process());
        shared_internals.complete(process);
        assert!(!shared_internals.scheduler.has_process());
        assert!(!shared_internals.scheduler.has_ready_process());
    }

    #[test]
    fn cloned_waker() {
        let shared_internals = new_internals();

        // Add a test process.
        let pid = add_process(&shared_internals.scheduler);
        assert!(shared_internals.scheduler.has_process());
        assert!(!shared_internals.scheduler.has_ready_process());

        // Create a cloned waker.
        let waker1 = waker::new(shared_internals.shared_id, pid);
        let waker2 = waker1.clone();
        drop(waker1);

        // Waking should move the process to the ready queue.
        waker2.wake();
        assert!(shared_internals.scheduler.has_process());
        assert!(shared_internals.scheduler.has_ready_process());
        let process = shared_internals.scheduler.remove().unwrap();
        assert_eq!(process.as_ref().id(), pid);
    }

    #[test]
    fn wake_from_different_thread() {
        let shared_internals = new_internals();

        let pid = add_process(&shared_internals.scheduler);
        assert!(shared_internals.scheduler.has_process());
        assert!(!shared_internals.scheduler.has_ready_process());

        let shared_internals2 = shared_internals.clone();
        let handle = thread::spawn(move || {
            let waker = waker::new(shared_internals2.shared_id, pid);
            waker.wake_by_ref();
            waker.wake();
        });

        loop {
            if let Some(process) = shared_internals.scheduler.remove() {
                assert_eq!(process.as_ref().id(), pid);
                shared_internals.complete(process);
                break;
            } else {
                sleep(Duration::from_millis(1));
                continue;
            }
        }

        handle.join().unwrap();
    }

    #[test]
    fn no_internals() {
        let waker_id = waker::init(Weak::new());
        let waker = waker::new(waker_id, PID1);

        // This shouldn't be a problem.
        waker.wake_by_ref();
        waker.wake();
    }

    #[test]
    fn will_wake() {
        let waker_id = waker::init(Weak::new());
        let waker1a = waker::new(waker_id, PID1);
        let waker1b = waker::new(waker_id, PID1);
        let waker2a = waker::new(waker_id, PID2);
        let waker2b = waker2a.clone();

        assert!(waker1a.will_wake(&waker1a));
        assert!(waker1a.will_wake(&waker1b));
        assert!(!waker1a.will_wake(&waker2a));
        assert!(!waker1a.will_wake(&waker2b));

        assert!(waker1b.will_wake(&waker1a));
        assert!(waker1b.will_wake(&waker1b));
        assert!(!waker1b.will_wake(&waker2a));
        assert!(!waker1b.will_wake(&waker2b));

        assert!(!waker2a.will_wake(&waker1a));
        assert!(!waker2a.will_wake(&waker1b));
        assert!(waker2a.will_wake(&waker2a));
        assert!(waker2a.will_wake(&waker2b));
    }

    fn new_internals() -> Arc<RuntimeInternals> {
        let poll = Poll::new().unwrap();
        let registry = poll.registry().try_clone().unwrap();
        let scheduler = Scheduler::new();
        let timers = Mutex::new(Timers::new());
        Arc::new_cyclic(|shared_internals| {
            let waker_id = waker::init(shared_internals.clone());
            let worker_wakers = vec![&*test::NOOP_WAKER].into_boxed_slice();
            RuntimeInternals::new(waker_id, worker_wakers, scheduler, registry, timers)
        })
    }

    fn add_process(scheduler: &Scheduler) -> ProcessId {
        let process: Pin<Box<dyn Process + Send + Sync>> = Box::pin(TestProcess);
        let process_data = Box::pin(ProcessData::new(Priority::NORMAL, process));
        let pid = process_data.as_ref().id();
        scheduler.add_process(process_data);
        pid
    }
}
