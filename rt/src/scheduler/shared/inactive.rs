use std::mem::replace;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::{fmt, ptr};

use crate::scheduler::shared::{ProcessData, RunQueue};
use crate::ProcessId;

/// Number of bits to shift per level.
const LEVEL_SHIFT: usize = 2;
/// Number of branches per level of the tree.
const N_BRANCHES: usize = 1 << LEVEL_SHIFT; // 4
/// Number of bits to mask per level.
const LEVEL_MASK: usize = (1 << LEVEL_SHIFT) - 1;
/// For alignment reasons the two least significant bits are always 0, so we can
/// safely skip them.
const SKIP_BITS: usize = 2;
const SKIP_MASK: usize = (1 << SKIP_BITS) - 1;

/// Returns `false` if `pid`'s `SKIP_BITS` aren't valid.
const fn ok_pid(pid: ProcessId) -> bool {
    pid.0 & SKIP_MASK == 0
}

/// Inactive processes.
///
/// Implemented as a tree with four pointers on each level. A pointer can be one
/// of four things:
///  * a pointer to a `Branch`, which again contains four pointers,
///  * a pointer to `ProcessData`, which hold the processes,
///  * a marker to indicate a process was marked as ready to run,
///  * or a null pointer to indicate the slot is empty.
///
/// Indexing into the structure is done using the `ProcessId` of the process,
/// however the pointer itself points to `ProcessData`.
///
/// Because processes should have short ready state times (see process states),
/// but longer total lifetime they quickly move into and out from this
/// structure. To ensure operations remain quick we keep the structure of tree
/// in place when removing processes. This also means that we don't have to
/// solve the difficult problem of concurrent garbage collection.
///
/// The implementation is effectively a hash set based on Hash Array Mapped Trie
/// (HAMT). Some resources:
/// * <https://en.wikipedia.org/wiki/Hash_array_mapped_trie>,
/// * <https://idea.popcount.org/2012-07-25-introduction-to-hamt>,
/// * Ideal Hash Trees by Phil Bagwell
/// * Fast And Space Efficient Trie Searches by Phil Bagwell
#[derive(Debug)]
pub(crate) struct Inactive {
    root: Branch,
    /// The number of processes in the tree, **not** markers.
    /// NOTE: do not use the value for correctness, it's highly likely to be
    /// outdated.
    length: AtomicUsize,
}

impl Inactive {
    /// Create an empty `Inactive` tree.
    pub(crate) const fn empty() -> Inactive {
        Inactive {
            root: Branch::empty(),
            length: AtomicUsize::new(0),
        }
    }

    /// Returns the number of processes in the inactive list.
    pub(crate) fn len(&self) -> usize {
        let len = self.length.load(Ordering::Relaxed);
        // The `length` can actually underflow quite easily, to not report a
        // clearly incorrect value we'll report zero instead.
        if len & (1 << 63) == 0 {
            len
        } else {
            0
        }
    }

    /// Returns `true` if the queue contains a process.
    ///
    /// # Notes
    ///
    /// Once this function returns the value could already be outdated.
    pub(crate) fn has_process(&self) -> bool {
        // NOTE: doing anything based on this function is prone to race
        // conditions, so relaxed ordering is fine.
        self.length.load(Ordering::Relaxed) != 0
    }

    /// Attempts to add the `process`.
    ///
    /// It will add `process` to `run_queue` if it was marked as ready-to-run
    /// while it was removed from the `Inactive` tree.
    pub(crate) fn add(&self, process: Pin<Box<ProcessData>>, run_queue: &RunQueue) {
        let pid = process.as_ref().id();
        debug_assert!(ok_pid(pid));
        let changed = self.root.add(process, pid.0 >> SKIP_BITS, 0, run_queue);
        self.update_length(changed);
    }

    /// Removes the process with id `pid`, if the process is currently not
    /// stored in the `Inactive` tree it is marked as ready and
    /// [`Inactive::add`] will return it once added back.
    pub(crate) fn mark_ready(&self, pid: ProcessId, run_queue: &RunQueue) {
        debug_assert!(ok_pid(pid));
        let changed = self.root.mark_ready(pid, pid.0 >> SKIP_BITS, 0, run_queue);
        self.update_length(changed);
    }

    /// Mark `process` as complete, removing a ready marker from the tree.
    pub(crate) fn complete(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        debug_assert!(ok_pid(pid));
        let ready_marker = ready_to_run(pid);

        let mut node = &self.root;
        let mut w_pid = pid.0 >> SKIP_BITS;
        // SAFETY: this needs to sync with all possible points that can change
        // this value; all need to use `Acquire`/`Release` (or `AcqRel`).
        let mut old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
        loop {
            if old_ptr == ready_marker {
                // Found the marker, try to remove it.
                // SAFETY: see comment for `load` above.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        // NOTE: we need to ensure the process lives until
                        // *after* we removed the marker to ensure we're not
                        // removing the marker for a process that was added
                        // after we dropped `process`, making us miss a wake-up
                        // for another process.
                        break;
                    }
                    // Another thread changed the pointer, try again with the
                    // updated (`old`) pointer.
                    Err(old) => old_ptr = old,
                }
            } else if is_branch(old_ptr) {
                // Pointer is a branch. Try at the next level.
                let branch_ptr: *mut Branch = as_ptr(old_ptr).cast();
                w_pid >>= LEVEL_SHIFT;
                debug_assert!(!branch_ptr.is_null());
                // SAFETY: if the pointer is a branch it must be always valid as
                // per the comment on `Branch.branches`.
                node = unsafe { &*branch_ptr };
                // SAFETY: see comment for `load` above.
                old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
            } else {
                // No marker for the process in the tree.
                break;
            }
        }

        // Don't want to panic when dropping the process.
        drop(catch_unwind(AssertUnwindSafe(move || drop(process))));
    }

    /// Update `length` with `n` added/removed processes.
    fn update_length(&self, n: isize) {
        #[allow(clippy::cast_sign_loss)]
        match n {
            0 => {}
            n if n.is_negative() => {
                // SAFETY: needs to sync with below.
                _ = self.length.fetch_sub(-n as usize, Ordering::AcqRel);
            }
            n => {
                // SAFETY: needs to sync with above.
                _ = self.length.fetch_add(n as usize, Ordering::AcqRel);
            }
        }
    }
}

struct Branch {
    /// Each pointer is a [`TaggedPointer`], see that type for valid values.
    ///
    /// Once the value of a pointer is set to point to a `Branch` they **must
    /// not** be changed to ensure the structure of the tree remains consistent.
    branches: [AtomicPtr<()>; N_BRANCHES],
}

/// An **owned**, tagged raw pointer.
/// Can be:
/// * `null`: empty.
/// * tagged with `PROCESS_TAG`: `Pin<Box<ProcessData>>`.
/// * tagged with `BRANCH_TAG`: `Pin<Box<Branch>>`.
/// * tagged with `READY_TO_RUN`: not a pointer, but a marker.
type TaggedPointer = *mut ();

/// Tags used for the `Pointer`.
const TAG_BITS: usize = 2;
const TAG_MASK: usize = (1 << TAG_BITS) - 1;
const BRANCH_TAG: usize = 0b01;
const PROCESS_TAG: usize = 0b10;
const READY_TO_RUN: usize = 0b11;

impl Branch {
    /// Create an empty `Branch`.
    const fn empty() -> Branch {
        #[allow(clippy::declare_interior_mutable_const)]
        const NONE: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());
        Branch {
            branches: [NONE; N_BRANCHES],
        }
    }

    /// Add `process` to the tree. Returns the number of processes added/removed
    /// from the tree.
    fn add(
        &self,
        process: Pin<Box<ProcessData>>,
        w_pid: usize,
        depth: usize,
        run_queue: &RunQueue,
    ) -> isize {
        let pid = process.as_ref().id();
        let process = tag_process(process);
        self._add(process, pid, w_pid, depth, run_queue)
    }

    fn _add(
        &self,
        process: TaggedPointer,
        pid: ProcessId,
        mut w_pid: usize,
        mut depth: usize,
        run_queue: &RunQueue,
    ) -> isize {
        debug_assert!(is_process(process));
        let mut node = self;
        // NOTE: from this point on `self` is invalid, use `node` instead.
        let mut old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
        let mut changed = 0;
        loop {
            if old_ptr.is_null() {
                // Empty slot, we can put the `process` into it.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    process,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return changed + 1,
                    // Another thread changed the pointer, try again with the
                    // updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            } else if is_branch(old_ptr) {
                // Pointer is a branch. Try at the next level.
                let branch_ptr: *mut Branch = as_ptr(old_ptr).cast();
                w_pid >>= LEVEL_SHIFT;
                depth += 1;
                debug_assert!(!branch_ptr.is_null());
                // SAFETY: checked if the pointer is a branch above and per the
                // docs of `Branch.branches` once it's a branch it's immutable.
                node = unsafe { &*branch_ptr };
                old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
            } else if is_ready_marker(old_ptr) && unsafe { as_pid(old_ptr) } == pid {
                // SAFETY: (above) `as_pid` is safe to call on ready markers.
                // Found a ready marker for the process we want to add.
                // Remove it and add the process to the run queue instead.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(old) => {
                        debug_assert!(is_ready_marker(old));
                        // SAFETY: per above `old` is a ready marker, thus it is
                        // safe to call `as_pid`.
                        debug_assert!(unsafe { as_pid(old) } == pid);
                        // SAFETY: caller must ensure `process` is tagged
                        // pointer to a process.
                        let process = unsafe { process_from_tagged(process) };
                        run_queue.add(process);
                        return changed;
                    }
                    // Another thread changed the pointer, try again with
                    // the updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            } else {
                debug_assert!(is_process(old_ptr) || is_ready_marker(old_ptr));
                // Found another process or ready marker for another process.
                // Remove it and create branch structure to hold both the
                // removed process/marker and `process`.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(other_process) => {
                        // Now we have to add two processes (or markers). First
                        // we create the branch structure that can hold the two
                        // processes, i.e. create enough branches to the point
                        // the two pointers differ in the branch slots. Required
                        // depth to go were the pointers are in different slots.
                        // SAFETY: we own `other_process` so we can safely call
                        // `as_pid`.
                        let other_pid = unsafe { as_pid(other_process) };
                        let req_depth = diff_branch_depth(other_pid, pid);
                        debug_assert!(req_depth > depth);
                        changed +=
                            node.add_branches(req_depth, ptr::null_mut(), w_pid, depth, run_queue);
                        // Add the other process/marker.
                        changed += if is_process(other_process) {
                            let w_pid = wpid_for(other_pid, depth);
                            // NOTE: `-1` because we've just removed the process
                            // above that we're going to add again here.
                            node._add(other_process, other_pid, w_pid, depth, run_queue) - 1
                        } else {
                            debug_assert!(is_ready_marker(other_process));
                            let w_pid = wpid_for(other_pid, depth);
                            node._mark_ready(other_process, w_pid, depth, run_queue)
                        };
                        // Continue our own adding process.
                        old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
                    }
                    // Another thread changed the pointer, try again with the
                    // updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            }
        }
    }

    /// Add a `marker` to the tree. Returns the number of processes
    /// added/removed from the tree.
    fn mark_ready(
        &self,
        pid: ProcessId,
        w_pid: usize,
        depth: usize,
        run_queue: &RunQueue,
    ) -> isize {
        let marker = ready_to_run(pid);
        self._mark_ready(marker, w_pid, depth, run_queue)
    }

    #[allow(clippy::cognitive_complexity)]
    fn _mark_ready(
        &self,
        marker: TaggedPointer,
        mut w_pid: usize,
        mut depth: usize,
        run_queue: &RunQueue,
    ) -> isize {
        debug_assert!(is_ready_marker(marker));
        // SAFETY: `as_pid` is safe to call with a ready marker.
        let marker_pid = unsafe { as_pid(marker) };
        let mut node = self;
        // NOTE: from this point on `self` is invalid, use `node` instead.
        let mut old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
        let mut changed = 0;
        loop {
            if old_ptr.is_null() {
                // Empty slot, we can put the `marker` into it.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    marker,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return changed,
                    // Another thread changed the pointer, try again with the
                    // updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            } else if is_branch(old_ptr) {
                // Pointer is a branch. Try at the next level.
                let branch_ptr: *mut Branch = as_ptr(old_ptr).cast();
                w_pid >>= LEVEL_SHIFT;
                depth += 1;
                debug_assert!(!branch_ptr.is_null());
                // SAFETY: checked if the pointer is a branch above and per the
                // docs of `Branch.branches` once it's a branch it's immutable.
                node = unsafe { &*branch_ptr };
                old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
            } else if is_ready_marker(old_ptr) && unsafe { as_pid(old_ptr) } == marker_pid {
                // SAFETY: (above) `as_pid` is safe to call on ready markers.
                // Already has a marker for the process.
                return changed;
            } else if is_process(old_ptr) && unsafe { as_pid(old_ptr) } == marker_pid {
                // SAFETY: (above) `as_pid` is safe to call on ready markers.
                // Already has a marker for the process.
                // Found the process, remove it.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        debug_assert!(is_process(old_ptr));
                        debug_assert!(!as_ptr(old_ptr).is_null());
                        // SAFETY: checked if the pointer is a process above.
                        let process = unsafe { process_from_tagged(old_ptr) };
                        run_queue.add(process);
                        return changed - 1;
                    }
                    // Another thread changed the pointer, try again with the
                    // updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            } else {
                debug_assert!(is_process(old_ptr) || is_ready_marker(old_ptr));
                // Found another process or ready marker for another process.
                // Remove it.
                match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                    old_ptr,
                    ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(other_process) => {
                        // Now we have to add two processes (or marker). First
                        // we create the branch structure that can hold the two
                        // processes, i.e. create enough branches to the point
                        // the two pointers differ in the branch slots.
                        debug_assert!(is_process(other_process) || is_ready_marker(other_process));
                        // SAFETY: we own `other_process` so we can safely call
                        // `as_pid`.
                        let other_pid = unsafe { as_pid(other_process) };
                        // Required depth to go were the pointers are in different slots.
                        let req_depth = diff_branch_depth(other_pid, marker_pid);
                        debug_assert!(req_depth > depth);
                        changed +=
                            node.add_branches(req_depth, ptr::null_mut(), w_pid, depth, run_queue);
                        // Add the other process/marker.
                        changed += if is_process(other_process) {
                            debug_assert!(is_process(other_process));
                            let w_pid = wpid_for(other_pid, depth);
                            // NOTE: `-1` because we've just removed the process
                            // above that we're going to add again here.
                            node._add(other_process, other_pid, w_pid, depth, run_queue) - 1
                        } else {
                            debug_assert!(is_ready_marker(other_process));
                            let w_pid = wpid_for(other_pid, depth);
                            node._mark_ready(other_process, w_pid, depth, run_queue)
                        };
                        // Continue our own adding process.
                        old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
                    }
                    // Another thread changed the pointer, try again with the
                    // updated (old) pointer.
                    Err(old) => old_ptr = old,
                }
            }
        }
    }

    /// Create branch structure so that the depth will be at least `req_depth`.
    /// Returns the number of processes added/removed from the tree.
    fn add_branches(
        &self,
        req_depth: usize,
        mut old_ptr: TaggedPointer,
        mut w_pid: usize,
        mut depth: usize,
        run_queue: &RunQueue,
    ) -> isize {
        // Build up to route to the branch.
        let mut node = self;
        // NOTE: from this point on `self` is invalid, use `node` instead.
        let mut w_branch = None;
        let mut changed = 0;
        while depth < req_depth {
            let branch = if let Some(branch) = w_branch.take() {
                debug_assert!(is_branch(branch));
                branch
            } else {
                let branch = Box::pin(Branch::empty());
                tag_branch(branch)
            };

            match node.branches[w_pid & LEVEL_MASK].compare_exchange(
                old_ptr,
                branch,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                // Success move to next depth.
                Ok(old) => {
                    debug_assert!(is_branch(branch));
                    let branch_ptr: *mut Branch = as_ptr(branch).cast();
                    w_pid >>= LEVEL_SHIFT;
                    depth += 1;
                    debug_assert!(!branch_ptr.is_null());
                    // SAFETY: created the branch pointer ourselves, so we know
                    // it's a branch.
                    node = unsafe { &*branch_ptr.cast() };
                    old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);

                    if is_process(old) {
                        debug_assert!(is_process(old));
                        // SAFETY: we own `old` so we can safely call `as_pid`.
                        let old_pid = unsafe { as_pid(old) };
                        let w_pid = wpid_for(old_pid, depth);
                        // NOTE: -1 because we've just removed the process.
                        changed += node._add(old, old_pid, w_pid, depth, run_queue) - 1;
                    } else if is_ready_marker(old) {
                        debug_assert!(is_ready_marker(old));
                        // SAFETY: `old` is a ready marker so it's safe to call.
                        let old_pid = unsafe { as_pid(old) };
                        let w_pid = wpid_for(old_pid, depth);
                        changed += node._mark_ready(old, w_pid, depth, run_queue);
                    } else {
                        debug_assert!(old_ptr.is_null());
                    }
                }
                // Another thread changed the pointer.
                Err(old) => {
                    // We failed to use `branch`, so we can use it again.
                    w_branch = Some(branch);
                    old_ptr = old;
                }
            }

            // Follow all branches created by other threads.
            while !as_ptr(old_ptr).is_null() && is_branch(old_ptr) {
                let branch_ptr: *mut Branch = as_ptr(old_ptr).cast();
                w_pid >>= LEVEL_SHIFT;
                depth += 1;
                debug_assert!(!branch_ptr.is_null());
                // SAFETY: checked if it's a branch pointer and non-null above.
                node = unsafe { &*branch_ptr.cast() };
                old_ptr = node.branches[w_pid & LEVEL_MASK].load(Ordering::Acquire);
            }
        }

        if let Some(branch) = w_branch {
            // SAFETY: created the pointer ourselves.
            unsafe { drop(branch_from_tagged(branch)) };
        }

        changed
    }
}

impl fmt::Debug for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn debug_pointer(ptr: &AtomicPtr<()>) -> &dyn fmt::Debug {
            let ptr = ptr.load(Ordering::Relaxed);
            match ptr as usize & TAG_MASK {
                _ if ptr.is_null() => &"null",
                BRANCH_TAG => {
                    let branch: *mut Branch = as_ptr(ptr).cast();
                    // SAFETY: check if it's a branch pointer.
                    unsafe { &*branch }
                }
                PROCESS_TAG => &"process",
                READY_TO_RUN => &"marker",
                _ => unreachable!(),
            }
        }

        f.debug_map()
            .entry(&"00", debug_pointer(&self.branches[0]))
            .entry(&"01", debug_pointer(&self.branches[1]))
            .entry(&"10", debug_pointer(&self.branches[2]))
            .entry(&"11", debug_pointer(&self.branches[3]))
            .finish()
    }
}

/// Returns the depth at which `pid1` and `pid2` use difference branches.
fn diff_branch_depth(pid1: ProcessId, pid2: ProcessId) -> usize {
    debug_assert!(pid1 != pid2);
    ((pid1.0 ^ pid2.0) >> SKIP_BITS).trailing_zeros() as usize / LEVEL_SHIFT
}

/// Converts `process` into a tagged pointer.
fn tag_process(process: Pin<Box<ProcessData>>) -> TaggedPointer {
    #[allow(trivial_casts)]
    let ptr = Box::into_raw(Pin::into_inner(process)).cast::<()>();
    (ptr as usize | PROCESS_TAG) as *mut ()
}

/// Tag a pointer as pointing to a branch.
fn tag_branch(branch: Pin<Box<Branch>>) -> TaggedPointer {
    #[allow(trivial_casts)]
    let ptr = Box::into_raw(Pin::into_inner(branch)).cast::<()>();
    (ptr as usize | BRANCH_TAG) as *mut ()
}

/// Create a mark ready-to-run `Pointer`.
const fn ready_to_run(pid: ProcessId) -> TaggedPointer {
    (pid.0 | READY_TO_RUN) as *mut ()
}

/// Convert a tagged pointer into a pointer to a process.
///
/// # Safety
///
/// Caller must ensure unique access to `ptr` and that it's a process.
unsafe fn process_from_tagged(ptr: TaggedPointer) -> Pin<Box<ProcessData>> {
    debug_assert!(is_process(ptr));
    Pin::new(Box::from_raw(as_ptr(ptr).cast()))
}

/// Convert a tagged pointer into a pointer to a branch.
///
/// # Safety
///
/// Caller must ensure unique access to `ptr` and that it's a branch.
unsafe fn branch_from_tagged(ptr: TaggedPointer) -> Pin<Box<Branch>> {
    debug_assert!(is_branch(ptr));
    Pin::new(Box::from_raw(as_ptr(ptr).cast()))
}

/// Returns `true` if the tagged pointer points to a branch.
fn is_branch(ptr: TaggedPointer) -> bool {
    has_tag(ptr, BRANCH_TAG)
}

/// Returns `true` if the tagged pointer points to a process.
fn is_process(ptr: TaggedPointer) -> bool {
    has_tag(ptr, PROCESS_TAG)
}

/// Returns `true` if the tagged pointer is a marker that the process is
/// ready-to-run.
fn is_ready_marker(ptr: TaggedPointer) -> bool {
    has_tag(ptr, READY_TO_RUN)
}

/// Returns `true` if the tagged pointer has a tag equal to `tag`.
fn has_tag(ptr: TaggedPointer, tag: usize) -> bool {
    (ptr as usize & TAG_MASK) == tag
}

/// Returns this pointer as `ProcessId`.
///
/// # Safety
///
/// This is only valid for ready-to-run markers or **owned** processes.
unsafe fn as_pid(ptr: TaggedPointer) -> ProcessId {
    if is_process(ptr) {
        Pin::new(unsafe { &*(as_ptr(ptr).cast::<ProcessData>()) }).id()
    } else {
        debug_assert!(is_ready_marker(ptr));
        ProcessId(as_ptr(ptr) as usize)
    }
}

/// Returns the raw pointer without its tag.
fn as_ptr(ptr: TaggedPointer) -> *mut () {
    (ptr as usize & !TAG_MASK) as *mut ()
}

/// Returns the working pid for `ptr` at `depth`.
const fn wpid_for(pid: ProcessId, depth: usize) -> usize {
    pid.0 >> ((depth * LEVEL_SHIFT) + SKIP_BITS)
}

impl Drop for Branch {
    fn drop(&mut self) {
        for ptr in &mut self.branches {
            let ptr = replace(ptr.get_mut(), ptr::null_mut());
            unsafe { drop_tagged_pointer(ptr) };
        }
    }
}

/// Drop a tagged pointer.
///
/// # Safety
///
/// Caller must ensure unique access to `ptr`.
unsafe fn drop_tagged_pointer(ptr: TaggedPointer) {
    if ptr.is_null() {
        return;
    }

    match ptr as usize & TAG_MASK {
        // SAFETY: checked for non-null and that it's a branch.
        BRANCH_TAG => drop(branch_from_tagged(ptr)),
        // SAFETY: checked for non-null and that it's a process.
        PROCESS_TAG => drop(process_from_tagged(ptr)),
        READY_TO_RUN => { /* Just a marker, nothing to drop. */ }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::mem::align_of;
    use std::pin::Pin;
    use std::ptr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{self, Poll};

    use crate::process::{Process, ProcessId};
    use crate::scheduler::shared::RunQueue;
    use crate::spawn::options::Priority;

    use super::{
        as_pid, branch_from_tagged, diff_branch_depth, drop_tagged_pointer, is_branch, is_process,
        is_ready_marker, process_from_tagged, ready_to_run, tag_branch, tag_process, Branch,
        Inactive, ProcessData, TaggedPointer,
    };

    #[test]
    fn pointer_is_send() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        // Required for `Pointer` to be `Send` and `Sync`.
        assert_send::<Pin<Box<ProcessData>>>();
        assert_sync::<Pin<Box<ProcessData>>>();
        assert_send::<Pin<Box<Branch>>>();
        assert_sync::<Pin<Box<Branch>>>();
        assert_send::<Inactive>();
        assert_sync::<Inactive>();
    }

    struct TestProcess;

    impl Future for TestProcess {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    impl Process for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }
    }

    fn test_process() -> Pin<Box<ProcessData>> {
        Box::pin(ProcessData::new(Priority::default(), Box::pin(TestProcess)))
    }

    #[test]
    fn size_assertions() {
        assert_eq!(size_of::<TaggedPointer>(), size_of::<usize>());
        assert_eq!(size_of::<Branch>(), 4 * size_of::<usize>());
    }

    #[test]
    fn process_data_alignment() {
        // Ensure that the pointer tag doesn't overwrite any pointer data.
        assert!(align_of::<ProcessData>() >= 2);
    }

    #[test]
    fn branch_alignment() {
        // Ensure that the pointer tag doesn't overwrite any pointer data.
        assert!(align_of::<Branch>() >= 2);
    }

    #[test]
    fn test_diff_branch_depth() {
        #[rustfmt::skip]
        let tests = &[
            (0b0000, 0b0100, 0),
            (0b0000, 0b1000, 0),
            (0b00_0000, 0b01_0000, 1),
            (0b00_0000, 0b10_0000, 1),
            (0b0000_0000_0000_0000_0000_0000, 0b0100_0000_0000_0000_0000_0000, 10),
            (0b0000_0000_0000_0000_0000_0000, 0b1000_0000_0000_0000_0000_0000, 10),
            (0b0000_0000_0000_0000_0000_0000, 0b0100_0000_0000_0000_0000_0100, 0),
            (0b0000_0000_0000_0000_0000_0000, 0b1000_0000_0000_0000_0000_1000, 0),
            // NOTE: first two bits, the `SKIP_BITS`, are ignored.
            (0b0000, 0b0101, 0),
            (0b0000, 0b1001, 0),
        ];
        for (pid1, pid2, expected) in tests.iter().copied() {
            let pid1 = ProcessId(pid1);
            let pid2 = ProcessId(pid2);
            let got = diff_branch_depth(pid1, pid2);
            assert_eq!(got, expected, "pid1: {pid1}, pid2: {pid2}");
        }
    }

    #[test]
    fn process_tagging() {
        let process = test_process();
        let tagged_process = tag_process(process);
        assert!(is_process(tagged_process));
        assert!(!is_branch(tagged_process));
        assert!(!is_ready_marker(tagged_process));
        let process = unsafe { process_from_tagged(tagged_process) };
        drop(process);
    }

    #[test]
    fn branch_tagging() {
        let branch = Box::pin(Branch::empty());
        let tagged_branch = tag_branch(branch);
        assert!(!is_process(tagged_branch));
        assert!(is_branch(tagged_branch));
        assert!(!is_ready_marker(tagged_branch));
        let branch = unsafe { branch_from_tagged(tagged_branch) };
        drop(branch);
    }

    #[test]
    fn ready_marker_tagging() {
        let pid = ProcessId(500);
        let tagged_pid = ready_to_run(pid);
        assert!(!is_process(tagged_pid));
        assert!(!is_branch(tagged_pid));
        assert!(is_ready_marker(tagged_pid));
        // SAFETY: `tagged_pid` is a ready marker so it's safe to call.
        let pid2 = unsafe { as_pid(tagged_pid) };
        assert_eq!(pid, pid2);
    }

    struct DropTest(Arc<AtomicUsize>);

    impl Drop for DropTest {
        fn drop(&mut self) {
            let _ = self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    impl Future for DropTest {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    impl Process for DropTest {
        fn name(&self) -> &'static str {
            "DropTest"
        }
    }

    #[test]
    fn dropping_tagged_process() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let process = Box::pin(DropTest(dropped.clone()));
        let process = Box::pin(ProcessData::new(Priority::default(), process));
        let ptr = tag_process(process);

        assert_eq!(dropped.load(Ordering::Acquire), 0);
        unsafe { drop_tagged_pointer(ptr) };
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn dropping_tagged_branch() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let process = Box::pin(DropTest(dropped.clone()));
        let process = Box::pin(ProcessData::new(Priority::default(), process));
        let process_ptr = tag_process(process);

        let mut branch = Box::pin(Branch::empty());
        *branch.branches[0].get_mut() = process_ptr;
        let branch_ptr = tag_branch(branch);

        assert_eq!(dropped.load(Ordering::Acquire), 0);
        unsafe { drop_tagged_pointer(branch_ptr) };
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn dropping_tagged_pid() {
        unsafe { drop_tagged_pointer(ready_to_run(ProcessId(500))) };
    }

    #[test]
    fn dropping_null_tagged_pointer() {
        unsafe { drop_tagged_pointer(ptr::null_mut()) };
    }

    #[test]
    fn marking_as_ready_to_run() {
        let tests = &[1, 2, 3, 4, 5, 100, 200];

        for n in tests {
            let tree = Inactive::empty();
            let run_queue = RunQueue::empty();

            let processes = (0..*n)
                .map(|_| {
                    let process = test_process();
                    let pid = process.as_ref().id();

                    // Process not in the tree, shouldn't be added to the run
                    // queue.
                    tree.mark_ready(pid, &run_queue);
                    assert!(!run_queue.has_process());

                    process
                })
                .collect::<Vec<_>>();

            for process in processes {
                let pid = process.as_ref().id();
                // Process should be marked as ready.
                tree.add(process, &run_queue);
                assert!(run_queue.has_process());
                let process = run_queue.remove().unwrap();
                assert_eq!(process.as_ref().id(), pid);
            }
        }
    }

    fn add_process(tree: &Inactive, run_queue: &RunQueue) -> ProcessId {
        assert!(!run_queue.has_process());
        let process = test_process();
        let pid = process.as_ref().id();
        tree.add(process, run_queue);
        assert!(!run_queue.has_process());
        pid
    }

    fn test(remove_order: Vec<usize>) {
        let tree = Inactive::empty();
        let run_queue = RunQueue::empty();
        let pids: Vec<ProcessId> = (0..remove_order.len())
            .map(|_| add_process(&tree, &run_queue))
            .collect();
        assert!(tree.has_process());
        println!(
            "After adding all {} processes: {:#?}",
            remove_order.len(),
            tree
        );

        let mut processes = Vec::with_capacity(pids.len());
        for index in remove_order {
            assert!(!run_queue.has_process());
            let pid = pids[index];
            // Marking the process as ready should add it to the run queue.
            tree.mark_ready(pid, &run_queue);
            let process = if let Some(p) = run_queue.remove() {
                p
            } else {
                panic!(
                    "failed to remove {}th process: pid={:064b} ({}), tree: {:#?}",
                    index + 1,
                    pid.0,
                    pid,
                    tree
                );
            };
            assert_eq!(process.as_ref().id(), pid);
            processes.push(process);

            // Can't add it to the run queue again.
            assert!(!run_queue.has_process());
            tree.mark_ready(pid, &run_queue);
            assert!(!run_queue.has_process());
        }
        assert!(!tree.has_process(), "tree: {:#?}", tree);

        for process in processes {
            tree.complete(process);
        }
        assert!(!tree.has_process());

        println!("Ok.");
    }

    // TODO: fix this.
    fn combinations(length: usize) -> Vec<Vec<usize>> {
        let mut all_indices: Vec<Vec<usize>> = Vec::new();
        for start in 0..length {
            let iter = (0..length).cycle().skip(start).take(length);
            all_indices.push(iter.collect());
        }

        for idx in 0..all_indices.len() {
            for i in 0..length - 1 {
                for j in i + 1..length {
                    let mut new = all_indices[idx].clone();
                    new.swap(i, j);
                    all_indices.push(new);
                }
            }
        }

        all_indices.sort();
        all_indices.dedup();

        all_indices
    }

    macro_rules! inactive_test {
        (all $name: ident, $n: expr) => {
            #[test]
            fn $name() {
                let n = $n;
                let remove_orders = combinations(n);
                for remove_order in remove_orders {
                    test(remove_order);
                }
            }
        };
        (one $name: ident, $n: expr) => {
            #[test]
            fn $name() {
                let n = $n;
                let remove_order = (0..n).collect();
                test(remove_order);
            }
        };
    }

    inactive_test!(all single_process, 1);
    inactive_test!(all two_processes, 2);
    inactive_test!(all three_processes, 3);
    inactive_test!(all four_processes, 4);
    inactive_test!(all five_processes, 5);
    inactive_test!(all ten_processes, 10);
    inactive_test!(one hundred_processes, 100);
    inactive_test!(one thousand_processes, 1000);
}
