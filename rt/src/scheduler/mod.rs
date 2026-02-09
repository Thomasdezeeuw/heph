//! Scheduler implementations.

use std::collections::BinaryHeap;
use std::mem::{self, replace, size_of};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{fmt, iter, ptr, task, thread};

use log::trace;

use crate::spawn::options::Priority;
use crate::worker::SYSTEM_ACTORS;

mod cfs;
pub(crate) mod process;
pub(crate) mod shared;
#[cfg(test)]
mod tests;

use cfs::Cfs;
pub(crate) use process::ProcessId;

type Process<S> = process::Process<S, dyn process::Run>;

/// Scheduling implementation.
///
/// The type itself holds the per process data needed for scheduling.
pub(crate) trait Schedule {
    /// Create new data.
    fn new(priority: Priority) -> Self;

    /// Update the process data with the latest run information.
    ///
    /// Arguments:
    ///  * `start`: time at which the latest run started.
    ///  * `end`: time at which the latest run ended.
    ///  * `elapsed`: `end - start`.
    fn update(&mut self, start: Instant, end: Instant, elapsed: Duration);

    /// Determine if the `lhs` or `rhs` should run first.
    fn order(lhs: &Self, rhs: &Self) -> std::cmp::Ordering;
}

#[derive(Debug)]
pub(crate) struct Scheduler<S: Schedule = Cfs> {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<Process<S>>>>,
    /// Processes that are not ready to run.
    inactive: Vec<Processes<S>>,
}

impl<S: Schedule> Scheduler<S> {
    /// Create a new `Scheduler`.
    pub(crate) fn new() -> Scheduler<S> {
        Scheduler {
            ready: BinaryHeap::with_capacity(8),
            inactive: Vec::with_capacity(8),
        }
    }

    /// Returns true if the scheduler has any processes that are ready to run.
    pub(crate) fn has_ready_process(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Returns true if the scheduler has any user processes (in any state),
    /// false otherwise. This ignore system processes.
    pub(crate) fn has_user_process(&self) -> bool {
        self.has_ready_process()
            || self
                .inactive
                .iter()
                .any(|p| usize::from(p.available) != (N_PROCESS_PER_GROUP - SYSTEM_ACTORS))
    }

    /// Returns the number of processes ready to run.
    pub(crate) fn ready(&self) -> usize {
        self.ready.len()
    }

    /// Returns the number of inactive processes.
    pub(crate) fn inactive(&self) -> usize {
        self.inactive.iter().map(|p| usize::from(p.available)).sum()
    }

    /// Mark all processes that are awoken as ready.
    pub(crate) fn ready_processes(&mut self) -> usize {
        let mut amount = 0;
        for inactive in self.inactive.iter_mut() {
            for index in inactive.bitmaps.set_iter() {
                if let Some(process) = inactive.processes[index as usize].mark_ready() {
                    self.ready.push(process);
                    amount += 1;
                }
            }
        }
        amount
    }

    /// Add a new proces to the scheduler.
    pub(crate) fn add_new_process<P>(&mut self, priority: Priority, process: P)
    where
        P: process::Run + 'static,
    {
        let pid = self.reserve_slot();
        let process = Box::pin(Process::<S>::new(pid, priority, Box::pin(process)));
        self.ready.push(process);
    }

    /// Reserve a slot for a process, returning the process id.
    fn reserve_slot(&mut self) -> ProcessId {
        for (n, inactive) in self.inactive.iter_mut().enumerate() {
            if inactive.available == 0 {
                continue;
            }
            for (idx, slot) in inactive
                .processes
                .iter_mut()
                .enumerate()
                .skip(inactive.next_empty as usize)
            {
                if !slot.is_empty() {
                    continue;
                }

                inactive.available -= 1;
                inactive.next_empty = idx as u16 + 1;
                slot.mark_empty_as_ready();

                return pid(n, idx as u16);
            }
        }

        let inactive = self.inactive.push_mut(Processes::new());
        inactive.available -= 1;
        inactive.next_empty = 1;
        inactive.processes[0].mark_empty_as_ready();

        return pid(self.inactive.len() - 1, 0);
    }

    /// Returns the next ready process.
    pub(crate) fn next_process(&mut self) -> Option<Pin<Box<Process<S>>>> {
        self.ready.pop()
    }

    /// Create a new waker for `process`.
    pub(crate) fn waker_for_process(&self, process: Pin<&Process<S>>) -> task::Waker {
        let (offset, idx) = offset_idx(process.id());
        self.inactive[offset].bitmaps.new_waker(idx)
    }

    /// Add back a `process` that was previously removed via
    /// [`Scheduler::next_process`].
    pub(crate) fn add_back_process(&mut self, process: Pin<Box<Process<S>>>) {
        let pid = process.id();
        trace!(pid; "adding back process");
        let (offset, idx) = offset_idx(process.id());
        if let Err(process) = self.inactive[offset].processes[idx as usize].add_back(process) {
            self.ready.push(process);
        }
    }

    /// Mark `process` as complete, removing it from the scheduler.
    ///
    /// Returns a possible panic (as error) from dropping the process. The
    /// scheduler is not affected by this.
    pub(crate) fn complete(&mut self, process: Pin<Box<Process<S>>>) -> thread::Result<()> {
        let pid = process.id();
        trace!(pid; "removing process");

        let (offset, idx) = offset_idx(process.id());
        let inactive = &mut self.inactive[offset];
        inactive.processes[idx as usize].mark_empty();
        inactive.available += 1;
        if inactive.next_empty > idx {
            inactive.next_empty = idx;
        }

        // Don't want to panic when dropping the process.
        catch_unwind(AssertUnwindSafe(move || drop(process)))
    }
}

/// Create a pid from the `offset` in `Scheduler::inactive` and the `idx` into
/// `Process:processes` (and `Processes:bitmaps`).
fn pid(offset: usize, idx: u16) -> ProcessId {
    ProcessId(offset << GROUP_SHIFT | idx as usize)
}

/// Reverse of [`pid`].
fn offset_idx(pid: ProcessId) -> (usize, u16) {
    let idx = (pid.0 & ((1 << GROUP_SHIFT) - 1)) as u16;
    let offset = pid.0 >> GROUP_SHIFT;
    (offset, idx)
}

/// Group of processes.
#[derive(Debug)]
struct Processes<S> {
    /// Process slots.
    processes: Box<[ProcessSlot<S>; N_PROCESS_PER_GROUP]>,
    /// Bitmaps indicate the processes are ready to run, but are not yet marked
    /// as such.
    bitmaps: ReadyMap,
    /// Index of the next empty process slot.
    next_empty: u16,
    /// Number of available slots.
    available: u16,
}

impl<S> Processes<S> {
    fn new() -> Processes<S> {
        /* TODO: use a const assertion once that works.
        const _ZEROED_OK: () =
        */
        debug_assert!(unsafe { mem::zeroed::<ProcessSlot::<()>>() }.is_empty());
        Processes {
            // SAFETY: all zero bits is valid for ProcessSlot as it means the
            // slot is empty.
            processes: unsafe { Box::new_zeroed().assume_init() },
            bitmaps: ReadyMap::new(),
            next_empty: 0,
            available: N_PROCESS_PER_GROUP as u16,
        }
    }
}

const N_PROCESS_PER_GROUP: usize = 8192 / size_of::<ProcessSlot<()>>();
const GROUP_SHIFT: usize = 10;
const _GROUP_SHIFT_OK: () = assert!(N_PROCESS_PER_GROUP <= (1 << GROUP_SHIFT));

struct ProcessSlot<S> {
    /// Pointer to the process, actual type is `Pin<Box<Process<S>>>`.
    ///
    /// This can have one of three states:
    ///  * waiting: `ptr` is valid.
    ///  * ready: `ptr` is dangling and the process is in the run queue.
    ///  * empty: `ptr` is null and the slot is not used.
    ///
    /// Or as an enum:
    ///
    /// enum ProcessSlot<S> {
    ///     Waiting(Pin<Box<Process<S>>>),
    ///     Ready,
    ///     Empty,
    /// }
    ///
    /// But that Rust can't fit that into a single pointer (at the time of
    /// writing).
    ptr: *mut Process<S>,
}

const _PROCESS_SLOT_SIZE: () = assert!(size_of::<ProcessSlot::<()>>() == size_of::<u64>());

impl<S> ProcessSlot<S> {
    const EMPTY: *mut Process<S> = ptr::null_mut(); // Not used.
    const READY: *mut Process<S> = 1 as *mut _; // In run queue.

    const fn is_empty(&self) -> bool {
        self.ptr.is_null()
    }

    fn is_waiting(&self) -> bool {
        !self.is_empty() && !self.is_ready()
    }

    fn is_ready(&self) -> bool {
        self.ptr == Self::READY
    }

    fn mark_empty_as_ready(&mut self) {
        debug_assert!(self.is_empty());
        self.ptr = Self::READY;
    }

    fn mark_ready(&mut self) -> Option<Pin<Box<Process<S>>>> {
        if self.is_waiting() {
            let ptr = replace(&mut self.ptr, Self::READY);
            // SAFETY: per the documentation on the ptr field, the pointer is
            // valid.
            Some(unsafe { Pin::new_unchecked(Box::from_raw(ptr)) })
        } else {
            None
        }
    }

    fn add_back(&mut self, process: Pin<Box<Process<S>>>) -> Result<(), Pin<Box<Process<S>>>> {
        debug_assert!(self.is_ready());
        // SAFETY: we take care of the pointer and don't move the process, see
        // the ptr field documentation.
        self.ptr = unsafe { Box::into_raw(Pin::into_inner_unchecked(process)) };
        Ok(())
    }

    fn mark_empty(&mut self) {
        debug_assert!(!self.is_waiting());
        self.ptr = Self::EMPTY;
    }
}

impl<S> fmt::Debug for ProcessSlot<S>
where
    Process<S>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            f.write_str("empty")
        } else if self.is_ready() {
            f.write_str("ready")
        } else {
            unsafe { &*self.ptr }.fmt(f)
        }
    }
}

impl<S> Drop for ProcessSlot<S> {
    fn drop(&mut self) {
        if self.is_waiting() {
            unsafe { drop(Pin::new_unchecked(Box::<Process<S>>::from_raw(self.ptr))) }
        }
    }
}

struct ReadyMap {
    /// Must always point to valid memory.
    ptr: NonNull<ReadyMapInner>,
}

// Using 4096 bytes in total for ReadyMapInner.
const N_BITS_CONTAINERS: usize = N_PROCESS_PER_GROUP / u64::BITS as usize;
const BITS_IN_MAP: usize = N_BITS_CONTAINERS * u64::BITS as usize;

struct ReadyMapInner {
    ref_count: AtomicU64,
    bits: [AtomicU64; N_BITS_CONTAINERS],
}

impl ReadyMap {
    fn new() -> ReadyMap {
        // SAFETY: all zero bits is valid for AtomicU64 and thus for
        // ReadyMapInner.
        let mut inner: Box<ReadyMapInner> = unsafe { Box::new_zeroed().assume_init() };
        *inner.ref_count.get_mut() = 1;
        ReadyMap {
            ptr: Box::into_non_null(inner),
        }
    }

    fn new_waker(&self, index: u16) -> task::Waker {
        static VTABLE: task::RawWakerVTable =
            task::RawWakerVTable::new(clone, wake, wake_by_ref, drop);

        const INDEX_SHIFT: usize = 64 - (u16::BITS as usize);
        const INDEX_MASK: usize = !((u16::MAX as usize) << INDEX_SHIFT);

        unsafe fn clone(data: *const ()) -> task::RawWaker {
            let (ptr, _) = untag(data);
            // SAFETY: in the creation of the task::Waker we've ensured the
            // pointer is valid.
            unsafe { ptr.as_ref() }.increase_ref_count();
            // SAFETY: we can reuse the same data (pointer and index are
            // unchanged).
            task::RawWaker::new(data, &VTABLE)
        }

        unsafe fn wake(data: *const ()) {
            let (ptr, idx) = untag(data);
            ReadyMap { ptr }.inner().set(idx);
        }

        unsafe fn wake_by_ref(data: *const ()) {
            let (ptr, idx) = untag(data);
            // SAFETY: in the creation of the task::Waker we've ensured the
            // pointer is valid.
            unsafe { ptr.as_ref() }.set(idx);
        }

        unsafe fn drop(data: *const ()) {
            let (ptr, _) = untag(data);
            mem::drop(ReadyMap { ptr });
        }

        fn tag(ptr: NonNull<ReadyMapInner>, index: u16) -> *const () {
            assert!(usize::from(index) <= BITS_IN_MAP);
            // We use the 16 upper bits that are unused in 64 bit addresses.
            ptr.as_ptr()
                .map_addr(|ptr| ptr | (usize::from(index) << INDEX_SHIFT))
                .cast()
        }

        fn untag(ptr: *const ()) -> (NonNull<ReadyMapInner>, u16) {
            let index = (ptr.addr() >> (INDEX_SHIFT)) as u16;
            let ptr = ptr.map_addr(|ptr| ptr & INDEX_MASK).cast_mut().cast();
            (unsafe { NonNull::new_unchecked(ptr) }, index)
        }

        let data = tag(self.ptr, index);
        let _ = self.inner().increase_ref_count(); // One for the new waker.
        unsafe { task::Waker::new(data, &VTABLE) }
    }

    /// Returns an iterator of bits set.
    ///
    /// NOTE: the iterator must be fully consumed or bits will be removed
    /// without processing.
    fn set_iter(&self) -> impl Iterator<Item = u16> {
        self.inner().bits.iter().enumerate().flat_map(|(n, data)| {
            // SAFETY: using Relaxed ordering first to check if the value is
            // not zero. If it's non-zero we use AcqRel load and overwrite
            // the value.
            let mut value = data.load(Ordering::Relaxed);
            if value != 0 {
                value = data.swap(0, Ordering::AcqRel);
            }
            let mut i = value.trailing_zeros();
            iter::from_fn(move || {
                while i < u64::BITS {
                    value &= !(1 << i);
                    if ((value >> i) & 1) != 0 {
                        return Some((n * usize::BITS as usize) as u16 + i as u16);
                    }
                    i += (value >> i).trailing_zeros();
                }
                None
            })
        })
    }

    fn inner(&self) -> &ReadyMapInner {
        // SAFETY: per the documentation on the ptr field it must always be
        // valid, so dereferencing it is safe.
        unsafe { self.ptr.as_ref() }
    }
}

impl ReadyMapInner {
    /// Set bit at `index`.
    fn set(&self, index: u16) {
        let index = usize::from(index);
        assert!(index <= BITS_IN_MAP);
        let idx = index / usize::BITS as usize;
        let n = index % usize::BITS as usize;
        if let Some(bits) = self.bits.get(idx) {
            _ = bits.fetch_or(1 << n, Ordering::AcqRel);
        }
    }

    fn increase_ref_count(&self) {
        // SAFETY: see Arc::clone for the safety on the atomic ordering, we
        // use the same logic here.
        let _ = self.ref_count.fetch_add(1, Ordering::Relaxed);
    }
}

impl Clone for ReadyMap {
    fn clone(&self) -> ReadyMap {
        self.inner().increase_ref_count();
        ReadyMap { ptr: self.ptr }
    }
}

impl fmt::Debug for ReadyMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner();
        f.debug_struct("ReadyMap")
            .field("ref_count", &inner.ref_count)
            .field("bits", &inner.bits)
            .finish()
    }
}

impl Drop for ReadyMap {
    fn drop(&mut self) {
        // SAFETY: this uses the same logic as Arc::drop.
        if self.inner().ref_count.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }

        unsafe {
            drop(Box::from_raw(self.ptr.as_ptr()));
        }
    }
}
