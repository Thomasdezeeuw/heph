use std::fmt;
use std::mem::{forget, replace};
use std::pin::Pin;
use std::ptr::NonNull;

use crate::rt::scheduler::shared::ProcessData;
use crate::rt::scheduler::ProcessId;

/// Number of branches per level of the tree, must be a power of 2.
const N_BRANCHES: usize = 4;
/// Number of bits to shift per level.
const LEVEL_SHIFT: usize = N_BRANCHES / 2;
/// Number of bits to mask per level.
const LEVEL_MASK: usize = (1 << LEVEL_SHIFT) - 1;
/// For alignment reasons the two least significant bits are always 0, so we can
/// safely skip them.
const SKIP_BITS: usize = 2;
const SKIP_MASK: usize = (1 << SKIP_BITS) - 1;

/// Returns `false` if `ptr`'s `SKIP_BITS` aren't valid.
pub(super) fn ok_ptr(ptr: *const ()) -> bool {
    ptr as usize & SKIP_MASK == 0
}

/// Inactive processes.
///
/// Implemented as a tree with four pointers on each level. A pointer can point
/// to a `Branch`, which again contains four pointers, or point to
/// `ProcessData`.
///
/// Because processes should have short ready state times (see process states),
/// but longer total lifetime they quickly move into and out from the structure.
/// To ensure operations remain quick we keep the structure of tree in place
/// when removing processes.
///
/// The implementation is effectively a hash set based on Hash array mapped trie
/// (HAMT). Some resources:
/// * https://en.wikipedia.org/wiki/Hash_array_mapped_trie,
/// * https://idea.popcount.org/2012-07-25-introduction-to-hamt,
/// * Ideal Hash Trees by Phil Bagwell
/// * Fast And Space Efficient Trie Searches by Phil Bagwell
// pub(in crate::rt) because its used in AddActor.
#[derive(Debug)]
pub(in crate::rt) struct Inactive {
    root: Branch,
    length: usize,
}

impl Inactive {
    /// Create an empty `Inactive` tree.
    pub(super) const fn empty() -> Inactive {
        Inactive {
            root: Branch::empty(),
            length: 0,
        }
    }

    /// Returns `true` if the queue contains a process.
    pub(super) fn has_process(&self) -> bool {
        self.length != 0
    }

    /// Attempts to add the `process`. Returns the `process` if it was marked as
    /// ready-to-run while it was removed from the `Inactive` list.
    pub(super) fn add(&mut self, process: Pin<Box<ProcessData>>) -> Option<Pin<Box<ProcessData>>> {
        let pid = process.as_ref().id();
        // Ensure `SKIP_BITS` is correct.
        debug_assert!(pid.0 & SKIP_MASK == 0);
        if let Some(process) = self.root.add(process, pid.0 >> SKIP_BITS, 0) {
            // Process was marked as ready-to-run.
            Some(process)
        } else {
            self.length += 1;
            None
        }
    }

    /// Removes the process with id `pid`, if the process is currently not
    /// stored in the `Inactive` list it is marked as ready and
    /// [`Inactive::add`] will return it once added back.
    pub(super) fn mark_ready(&mut self, pid: ProcessId) -> Option<Pin<Box<ProcessData>>> {
        debug_assert!(ok_ptr(pid.0 as *mut ()));

        self.root
            .mark_ready(pid, pid.0 >> SKIP_BITS, 0)
            .map(|process| {
                debug_assert_eq!(process.as_ref().id(), pid);
                self.length -= 1;
                process
            })
    }
}

struct Branch {
    branches: [Option<Pointer>; N_BRANCHES],
}

impl fmt::Debug for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entry(&("00", &self.branches[0]))
            .entry(&("01", &self.branches[1]))
            .entry(&("10", &self.branches[2]))
            .entry(&("11", &self.branches[3]))
            .finish()
    }
}

impl Branch {
    /// Create an empty `Branch`.
    const fn empty() -> Branch {
        Branch {
            branches: [None, None, None, None],
        }
    }

    fn add(
        &mut self,
        process: Pin<Box<ProcessData>>,
        w_pid: usize,
        depth: usize,
    ) -> Option<Pin<Box<ProcessData>>> {
        let node = &mut self.branches[w_pid & LEVEL_MASK];
        match Pointer::take(node) {
            Some(Pointee::Process(other_process)) => {
                // This branch ends in another process, add them both.
                self.add_both(other_process.into(), process.into(), w_pid, depth);
                None
            }
            Some(Pointee::Branch(mut branch)) => {
                // Try at the next level.
                branch.add(process, w_pid >> LEVEL_SHIFT, depth + 1)
            }
            Some(Pointee::ReadyToRun(other_pid)) if process.as_ref().id() == other_pid => {
                // Process was marked as ready to run, so return it.
                *node = None;
                Some(process)
            }
            Some(Pointee::ReadyToRun(other_pid)) => {
                let other = Pointer::ready_to_run(other_pid);
                *node = None;
                self.add_both(other, process.into(), w_pid, depth);
                None
            }
            None => {
                self.branches[w_pid & LEVEL_MASK] = Some(process.into());
                None
            }
        }
    }

    /// Add two pointers at a time.
    ///
    /// Used when a process is added but another process or marker is
    /// encountered. Instead of adding one process (by creating a new level) and
    /// then add the other, possibly creating another conflict. We first create
    /// enough levels to not have conflicting branches and then add both
    /// processes.
    ///
    /// # Notes
    ///
    /// Both `ptr1` and `ptr2` must be of kind process or ready-to-run marker.
    fn add_both(&mut self, ptr1: Pointer, ptr2: Pointer, w_pid: usize, depth: usize) {
        debug_assert!(self.branches[w_pid & LEVEL_MASK].is_none());
        debug_assert!(ptr1.is_process() || ptr1.is_ready_marker());
        debug_assert!(ptr2.is_process() || ptr2.is_ready_marker());

        let pid1 = ptr1.as_pid();
        let pid2 = ptr2.as_pid();

        // Build the part of the branch in reverse, starting at the lowest
        // branch.
        let mut branch = Box::pin(Branch::empty());
        // Required depth to go were the pointers are in different slots.
        let req_depth = diff_branch_depth(pid1, pid2, depth);
        // Add the two pointers.
        let w_pid1 = skip_bits(pid1, depth + req_depth);
        branch.branches[w_pid1 & LEVEL_MASK] = Some(ptr1);
        let w_pid2 = skip_bits(pid2, depth + req_depth);
        branch.branches[w_pid2 & LEVEL_MASK] = Some(ptr2);
        debug_assert!(w_pid1 & LEVEL_MASK != w_pid2 & LEVEL_MASK);
        // Build up to route to the branch.
        for depth in 1..req_depth {
            let index = (w_pid >> ((req_depth - depth) * LEVEL_SHIFT)) & LEVEL_MASK;
            let next_branch = replace(&mut branch, Box::pin(Branch::empty()));
            branch.branches[index] = Some(next_branch.into());
        }
        self.branches[w_pid & LEVEL_MASK] = Some(branch.into());
    }

    fn mark_ready(
        &mut self,
        pid: ProcessId,
        w_pid: usize,
        depth: usize,
    ) -> Option<Pin<Box<ProcessData>>> {
        let node = &mut self.branches[w_pid & LEVEL_MASK];
        match Pointer::take(node) {
            Some(Pointee::Process(process)) if process.as_ref().id() == pid => Some(process),
            Some(Pointee::Process(process)) => {
                // Found another process, create a ready-to-run marker and add
                // the marker and the process both.
                let marker = Pointer::ready_to_run(pid);
                self.add_both(process.into(), marker, w_pid, depth);
                None
            }
            Some(Pointee::Branch(mut next_branch)) => {
                next_branch.mark_ready(pid, w_pid >> LEVEL_SHIFT, depth + 1)
            }
            Some(Pointee::ReadyToRun(other_pid)) if pid == other_pid => {
                // Already has a ready-to-run marker, don't have to do anything.
                None
            }
            Some(Pointee::ReadyToRun(other_pid)) => {
                let other = Pointer::ready_to_run(other_pid);
                let marker = Pointer::ready_to_run(pid);
                *node = None;
                self.add_both(other, marker, w_pid, depth);
                None
            }
            None => {
                *node = Some(Pointer::ready_to_run(pid));
                None
            }
        }
    }
}

/// Tagged pointer to either a `Branch` or `ProcessData`.
struct Pointer {
    /// This is actually either a `Pin<Box<ProcessData>>` or `Pin<Box<Branch>>`.
    tagged_ptr: NonNull<()>,
}

/// Tags used for the `Pointer`.
const TAG_BITS: usize = 0b11;
const READY_TO_RUN: usize = 0b01;
const PROCESS_TAG: usize = 0b10;
const BRANCH_TAG: usize = 0b00;

/// Returned by [`Pointer::take`].
enum Pointee<'a> {
    Process(Pin<Box<ProcessData>>),
    Branch(Pin<&'a mut Branch>),
    ReadyToRun(ProcessId),
}

impl Pointer {
    /// Create a mark ready-to-run `Pointer`.
    fn ready_to_run(pid: ProcessId) -> Pointer {
        debug_assert!(ok_ptr(pid.0 as *mut ()));
        let ptr = (pid.0 | READY_TO_RUN) as *mut ();
        Pointer {
            tagged_ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }

    /// Attempts to take this pointer if its a process or ready-to-run marker.
    fn take<'a>(this: &'a mut Option<Pointer>) -> Option<Pointee<'a>> {
        match this {
            Some(pointer) if pointer.is_ready_marker() => {
                Some(Pointee::ReadyToRun(pointer.as_pid()))
            }
            Some(pointer) if pointer.is_process() => {
                let p = unsafe { Box::from_raw(pointer.as_ptr() as *mut _) };
                // We just read the pointer so now we have to forget it.
                forget(this.take());
                Some(Pointee::Process(Pin::new(p)))
            }
            Some(pointer) => {
                debug_assert!(!pointer.is_process());
                let p: &mut Branch = unsafe { &mut *(pointer.as_ptr() as *mut _) };
                Some(Pointee::Branch(unsafe { Pin::new_unchecked(p) }))
            }
            None => None,
        }
    }

    /// Returns the raw pointer without its tag.
    fn as_ptr(&self) -> *mut () {
        (self.tagged_ptr.as_ptr() as usize & !TAG_BITS) as *mut ()
    }

    /// Returns this pointer `ProcessId`.
    ///
    /// # Notes
    ///
    /// This is only valid for process pointers and ready-to-run markers.
    fn as_pid(&self) -> ProcessId {
        ProcessId(self.tagged_ptr.as_ptr() as usize & !TAG_BITS)
    }

    /// Returns `true` is the tagged pointer is a marker that the process is
    /// ready-to-run.
    fn is_ready_marker(&self) -> bool {
        (self.tagged_ptr.as_ptr() as usize & READY_TO_RUN) != 0
    }

    /// Returns `true` is the tagged pointer points to a process.
    fn is_process(&self) -> bool {
        (self.tagged_ptr.as_ptr() as usize & PROCESS_TAG) != 0
    }
}

/// This is safe because `Pin<Box<ProcessData>>` and `Pin<Box<Branch>>` are
/// `Send` and `Sync`.
unsafe impl Send for Pointer {}
unsafe impl Sync for Pointer {}

impl From<Pin<Box<ProcessData>>> for Pointer {
    fn from(process: Pin<Box<ProcessData>>) -> Pointer {
        #[allow(trivial_casts)]
        let ptr = Box::leak(Pin::into_inner(process)) as *mut _;
        let ptr = (ptr as usize | PROCESS_TAG) as *mut ();
        Pointer {
            tagged_ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }
}

impl From<Pin<Box<Branch>>> for Pointer {
    fn from(process: Pin<Box<Branch>>) -> Pointer {
        #[allow(trivial_casts)]
        let ptr = Box::leak(Pin::into_inner(process)) as *mut _;
        let ptr = (ptr as usize | BRANCH_TAG) as *mut ();
        Pointer {
            tagged_ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        let ptr = self.as_ptr();
        if self.is_ready_marker() {
            // Nothing to do.
        } else if self.is_process() {
            let p: Box<ProcessData> = unsafe { Box::from_raw(ptr as *mut _) };
            drop(p);
        } else {
            let p: Box<Branch> = unsafe { Box::from_raw(ptr as *mut _) };
            drop(p);
        }
    }
}

impl fmt::Debug for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ptr = self.as_ptr();
        if self.is_process() {
            let p: &ProcessData = unsafe { &mut *(ptr as *mut _) };
            let p: Pin<&ProcessData> = unsafe { Pin::new_unchecked(p) };
            p.fmt(f)
        } else {
            let p: &Branch = unsafe { &mut *(ptr as *mut _) };
            let p: Pin<&Branch> = unsafe { Pin::new_unchecked(p) };
            p.fmt(f)
        }
    }
}

/// Returns the depth at which `pid1` and `pid2` use difference branches.
///
/// # Notes
///
/// This loops forever if `pid1` and `pid2` are equal.
fn diff_branch_depth(pid1: ProcessId, pid2: ProcessId, depth: usize) -> usize {
    debug_assert!(pid1 != pid2);
    let mut pid1 = skip_bits(pid1, depth);
    let mut pid2 = skip_bits(pid2, depth);
    debug_assert!(pid1 != pid2);
    let mut depth = 0;
    while pid1 & LEVEL_MASK == pid2 & LEVEL_MASK {
        debug_assert!(depth < 32);
        debug_assert!(pid1 > 0);
        depth += 1;
        pid1 >>= LEVEL_SHIFT;
        pid2 >>= LEVEL_SHIFT;
    }
    depth
}

/// Skip the bits up to `depth`.
fn skip_bits(pid: ProcessId, depth: usize) -> usize {
    pid.0 >> ((depth * LEVEL_SHIFT) + SKIP_BITS)
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::rt::process::{Process, ProcessId, ProcessResult};
    use crate::rt::scheduler::Priority;
    use crate::rt::RuntimeRef;

    use super::{Branch, Inactive, Pointee, Pointer, ProcessData};

    #[test]
    fn pointer_is_send() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<Pointer>();
        assert_sync::<Pointer>();
        // Required for `Pointer` to be `Send` and `Sync`.
        assert_send::<Pin<Box<ProcessData>>>();
        assert_sync::<Pin<Box<ProcessData>>>();
        assert_send::<Pin<Box<Branch>>>();
        assert_sync::<Pin<Box<Branch>>>();
    }

    struct TestProcess;

    impl Process for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }

        fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
            unimplemented!()
        }
    }

    fn test_process() -> Pin<Box<ProcessData>> {
        Box::pin(ProcessData::new(Priority::default(), Box::pin(TestProcess)))
    }

    #[test]
    fn size_assertions() {
        assert_eq!(size_of::<Pointer>(), size_of::<usize>());
        assert_eq!(size_of::<Branch>(), 4 * size_of::<usize>());
    }

    #[test]
    fn process_data_alignment() {
        // Ensure that the pointer tag doesn't overwrite any pointer data.
        assert!(align_of::<ProcessData>() >= 2);
        assert!(align_of::<Branch>() >= 2);
    }

    #[test]
    fn branch_alignment() {
        // Ensure that the pointer tag doesn't overwrite any pointer data.
        assert!(align_of::<Branch>() >= 2);
    }

    #[test]
    fn pointer_take() {
        // None.
        let mut ptr: Option<Pointer> = None;
        assert!(Pointer::take(&mut ptr).is_none());
        assert!(ptr.is_none());

        // Process.
        let mut ptr: Option<Pointer> = Some(test_process().into());
        match Pointer::take(&mut ptr) {
            Some(Pointee::Process(_)) => {}
            _ => panic!("unexpected result"),
        }
        // Process is removed.
        assert!(ptr.is_none());

        // Branch.
        let mut ptr: Option<Pointer> = Some(Box::pin(Branch::empty()).into());
        match Pointer::take(&mut ptr) {
            Some(Pointee::Branch(_)) => {}
            _ => panic!("unexpected result"),
        }
        // Pointer unchanged.
        assert!(ptr.is_some());

        // Ready-to-run marker.
        let pid = ProcessId(1 << 4);
        let mut ptr: Option<Pointer> = Some(Pointer::ready_to_run(pid));
        match Pointer::take(&mut ptr) {
            Some(Pointee::ReadyToRun(p)) => assert_eq!(p, pid),
            _ => panic!("unexpected result"),
        }
        // Pointer unchanged.
        assert!(ptr.is_some());
    }

    #[test]
    fn pointer_as_pid() {
        // Process.
        let process = test_process();
        let pid = process.as_ref().id();
        let got = Pointer::from(process).as_pid();
        assert_eq!(pid, got);

        // Ready-to-run marker.
        let pid = ProcessId(1 << 4);
        let ptr = Pointer::ready_to_run(pid);
        assert_eq!(ptr.as_pid(), pid);
    }

    #[test]
    fn pointer_is_marker_ready() {
        // Process.
        let process = Pointer::from(test_process());
        assert!(!process.is_ready_marker());

        // Branch.
        let branch = Pointer::from(Box::pin(Branch::empty()));
        assert!(!branch.is_ready_marker());

        // Ready-to-run marker.
        let pid = ProcessId(1 << 4);
        let marker = Pointer::ready_to_run(pid);
        assert!(marker.is_ready_marker());
    }

    #[test]
    fn drop_test_branch() {
        // This shouldn't panic or anything.
        let ptr: Pointer = Box::pin(Branch::empty()).into();
        drop(ptr);
    }

    #[test]
    fn drop_test_process() {
        // This shouldn't panic or anything.
        struct DropTest(Arc<AtomicUsize>);

        impl Drop for DropTest {
            fn drop(&mut self) {
                let _ = self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        impl Process for DropTest {
            fn name(&self) -> &'static str {
                "DropTest"
            }

            fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
                unimplemented!()
            }
        }

        let dropped = Arc::new(AtomicUsize::new(0));

        let process = Box::pin(DropTest(dropped.clone()));
        let ptr: Pointer = Box::pin(ProcessData::new(Priority::default(), process)).into();

        assert_eq!(dropped.load(Ordering::Acquire), 0);
        drop(ptr);
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn marking_as_ready_to_run() {
        let tests = &[1, 2, 3, 4, 5, 100, 200];

        for n in tests {
            let mut queue = Inactive::empty();

            let processes = (0..*n)
                .map(|_| {
                    let process = test_process();
                    let pid = process.as_ref().id();

                    // Process not in the queue.
                    assert!(queue.mark_ready(pid).is_none());
                    process
                })
                .collect::<Vec<_>>();

            for process in processes {
                // Process should be marked as ready.
                assert!(queue.add(process).is_some());
            }
        }
    }

    fn add_process(queue: &mut Inactive) -> ProcessId {
        let process = test_process();
        let pid = process.as_ref().id();
        let mut p = Some(process);
        // Remove any ready-to-run markers.
        while let Some(process) = p.take() {
            p = queue.add(process);
        }
        pid
    }

    fn test(remove_order: Vec<usize>) {
        let mut queue = Inactive::empty();
        let pids: Vec<ProcessId> = (0..remove_order.len())
            .map(|_| add_process(&mut queue))
            .collect();
        println!(
            "queue after adding all {} processes: {:#?}",
            remove_order.len(),
            queue
        );

        for index in remove_order {
            let pid = pids[index];
            let process = if let Some(p) = queue.mark_ready(pid) {
                p
            } else {
                panic!(
                    "failed to remove {}th process: pid={:064b} ({}), queue: {:#?}",
                    index + 1,
                    pid.0,
                    pid,
                    queue
                );
            };
            assert_eq!(process.as_ref().id(), pid);
            assert!(queue.mark_ready(pid).is_none());
        }
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
