use std::cmp::max;
use std::fmt;
use std::mem::{forget, replace};
use std::pin::Pin;
use std::ptr::NonNull;

use crate::process::ProcessId;
use crate::scheduler::ProcessData;
use crate::worker::SYSTEM_ACTORS;

/// Number of bits to shift per level.
const LEVEL_SHIFT: usize = 4;
/// Number of branches per level of the tree, must be a power of 2.
const N_BRANCHES: usize = 1 << LEVEL_SHIFT; // 16
/// Number of bits to mask per level.
const LEVEL_MASK: usize = (1 << LEVEL_SHIFT) - 1;
/// For alignment reasons the two least significant bits of a boxed
/// `ProcessData` are always 0, so we can safely skip them. Also see `ok_pid`
/// and alignment tests below.
const SKIP_BITS: usize = 2;
const SKIP_MASK: usize = (1 << SKIP_BITS) - 1;

/// Returns `false` if `pid`'s `SKIP_BITS` aren't valid.
fn ok_pid(pid: ProcessId) -> bool {
    pid.0 & max(SKIP_MASK, POINTER_TAG_BITS) == 0
}

/// Inactive processes.
///
/// Implemented as a tree with four pointers on each level. A pointer can point
/// to a `Branch`, which again contains four pointers, or point to
/// `ProcessData`.
///
/// Indexing into the structure is done using the `ProcessId` of the process,
/// however the pointer itself points to `ProcessData`.
///
/// Because processes should have short ready state times (see process states),
/// but longer total lifetime they quickly move into and out from the structure.
/// To ensure operations remain quick we keep the structure of tree in place
/// when removing processes.
///
/// The implementation is effectively a hash set based on Hash array mapped trie
/// (HAMT). Some resources:
/// * <https://en.wikipedia.org/wiki/Hash_array_mapped_trie>,
/// * <https://idea.popcount.org/2012-07-25-introduction-to-hamt>,
/// * Ideal Hash Trees by Phil Bagwell
/// * Fast And Space Efficient Trie Searches by Phil Bagwell
#[derive(Debug)]
pub(crate) struct Inactive {
    root: Branch,
    length: usize,
}

impl Inactive {
    /// Create an empty `Inactive` tree.
    pub(crate) const fn empty() -> Inactive {
        Inactive {
            root: Branch::empty(),
            length: 0,
        }
    }

    /// Returns the number of processes in the inactive list.
    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    /// Returns `true` if the list contains a user process.
    pub(crate) fn has_user_process(&self) -> bool {
        self.length > SYSTEM_ACTORS
    }

    /// Add a `process`.
    pub(crate) fn add(&mut self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        debug_assert!(ok_pid(pid));
        self.root.add(process, pid.0 >> SKIP_BITS, 0);
        self.length += 1;
    }

    /// Removes the process with id `pid`, if any.
    pub(crate) fn remove(&mut self, pid: ProcessId) -> Option<Pin<Box<ProcessData>>> {
        debug_assert!(ok_pid(pid));
        self.root
            .remove(pid, pid.0 >> SKIP_BITS)
            .inspect(|process| {
                debug_assert_eq!(process.as_ref().id(), pid);
                self.length -= 1;
            })
    }
}

struct Branch {
    branches: [Option<Pointer>; N_BRANCHES],
}

impl fmt::Debug for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entry(&"0000", &self.branches[0])
            .entry(&"0001", &self.branches[1])
            .entry(&"0010", &self.branches[2])
            .entry(&"0011", &self.branches[3])
            .entry(&"0100", &self.branches[4])
            .entry(&"0101", &self.branches[5])
            .entry(&"0110", &self.branches[6])
            .entry(&"0111", &self.branches[7])
            .entry(&"1000", &self.branches[8])
            .entry(&"1001", &self.branches[9])
            .entry(&"1010", &self.branches[10])
            .entry(&"1011", &self.branches[11])
            .entry(&"1100", &self.branches[12])
            .entry(&"1101", &self.branches[13])
            .entry(&"1110", &self.branches[14])
            .entry(&"1111", &self.branches[15])
            .finish()
    }
}

impl Branch {
    /// Create an empty `Branch`.
    const fn empty() -> Branch {
        const NONE: Option<Pointer> = None;
        Branch {
            branches: [NONE; N_BRANCHES],
        }
    }

    fn add(&mut self, process: Pin<Box<ProcessData>>, w_pid: usize, depth: usize) {
        match Pointer::take_process(&mut self.branches[w_pid & LEVEL_MASK]) {
            Some(Ok(other_process)) => self.add_both(process, other_process, w_pid, depth),
            Some(Err(mut branch)) => branch.add(process, w_pid >> LEVEL_SHIFT, depth + 1),
            None => self.branches[w_pid & LEVEL_MASK] = Some(process.into()),
        }
    }

    /// Add two processes at a time.
    ///
    /// Used when a process is added but another process is encountered at the
    /// end of a branch. Instead of adding one process (by creating a new level)
    /// and then add the other, possibly creating another conflict. We first
    /// create enough levels to not have conflicting branches and then add both
    /// processes.
    fn add_both(
        &mut self,
        process1: Pin<Box<ProcessData>>,
        process2: Pin<Box<ProcessData>>,
        w_pid: usize,
        depth: usize,
    ) {
        debug_assert!(self.branches[w_pid & LEVEL_MASK].is_none());
        let pid1 = process1.as_ref().id();
        let pid2 = process2.as_ref().id();

        // Build the part of the branch in reverse, starting at the lowest
        // branch.
        let mut branch = Box::pin(Branch::empty());
        // Required depth to go were the pointers are in different slots.
        let req_depth = diff_branch_depth(pid1, pid2);
        debug_assert!(req_depth >= depth);
        // Add the two processes.
        let w_pid1 = skip_bits(pid1, req_depth);
        branch.branches[w_pid1 & LEVEL_MASK] = Some(process1.into());
        let w_pid2 = skip_bits(pid2, req_depth);
        branch.branches[w_pid2 & LEVEL_MASK] = Some(process2.into());
        debug_assert!(w_pid1 & LEVEL_MASK != w_pid2 & LEVEL_MASK);
        // Build up to route to the branch.
        for depth in depth + 1..req_depth {
            let index = (w_pid >> ((req_depth - depth) * LEVEL_SHIFT)) & LEVEL_MASK;
            let next_branch = replace(&mut branch, Box::pin(Branch::empty()));
            branch.branches[index] = Some(next_branch.into());
        }
        self.branches[w_pid & LEVEL_MASK] = Some(branch.into());
    }

    fn remove(&mut self, pid: ProcessId, w_pid: usize) -> Option<Pin<Box<ProcessData>>> {
        let node = &mut self.branches[w_pid & LEVEL_MASK];
        match Pointer::take_process(node) {
            Some(Ok(process)) if process.as_ref().id() == pid => Some(process),
            Some(Ok(process)) => {
                // Wrong process so put it back.
                *node = Some(process.into());
                None
            }
            Some(Err(mut next_branch)) => next_branch.remove(pid, w_pid >> LEVEL_SHIFT),
            None => None,
        }
    }
}

/// Tagged pointer to either a `Branch` or `ProcessData`.
struct Pointer {
    /// This is actually either a `Pin<Box<ProcessData>>` or `Pin<Box<Branch>>`.
    tagged_ptr: NonNull<()>,
}

/// Number of bits used for the tag in `Pointer`.
const POINTER_TAG_BITS: usize = 1;
/// Tags used for the `Pointer`.
const PROCESS_TAG: usize = 0b1;
const BRANCH_TAG: usize = 0b0;

impl Pointer {
    /// Attempts to take a process pointer from `this`, or returns a mutable
    /// reference to the branch.
    ///
    /// Returns:
    /// * `None` if `this` is `None`, `this` is unchanged.
    /// * `Some(Ok(..))` if the pointer is `Some` and points to a process,
    ///    `this` will be `None`.
    /// * `Some(Err(..))` if the pointer is `Some` and points to a branch,
    ///    `this` is unchanged.
    fn take_process<'a>(
        this: &'a mut Option<Pointer>,
    ) -> Option<Result<Pin<Box<ProcessData>>, Pin<&'a mut Branch>>> {
        match this {
            Some(pointer) if pointer.is_process() => {
                let p = unsafe { Box::from_raw(pointer.as_ptr().cast()) };
                // We just read the pointer so now we have to forget it.
                forget(this.take());
                Some(Ok(Pin::new(p)))
            }
            Some(pointer) => {
                debug_assert!(!pointer.is_process());
                let p: &mut Branch = unsafe { &mut *(pointer.as_ptr().cast()) };
                Some(Err(unsafe { Pin::new_unchecked(p) }))
            }
            None => None,
        }
    }

    /// Returns the raw pointer without its tag.
    fn as_ptr(&self) -> *mut () {
        (self.tagged_ptr.as_ptr() as usize & !(PROCESS_TAG)) as *mut ()
    }

    /// Returns `true` is the tagged pointer points to a process.
    fn is_process(&self) -> bool {
        (self.tagged_ptr.as_ptr() as usize & PROCESS_TAG) != 0
    }
}

impl From<Pin<Box<ProcessData>>> for Pointer {
    fn from(process: Pin<Box<ProcessData>>) -> Pointer {
        #[allow(trivial_casts)]
        let ptr = Box::into_raw(Pin::into_inner(process));
        let ptr = (ptr as usize | PROCESS_TAG) as *mut ();
        Pointer {
            tagged_ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }
}

impl From<Pin<Box<Branch>>> for Pointer {
    fn from(process: Pin<Box<Branch>>) -> Pointer {
        #[allow(trivial_casts)]
        let ptr = Box::into_raw(Pin::into_inner(process));
        let ptr = (ptr as usize | BRANCH_TAG) as *mut ();
        Pointer {
            tagged_ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        let ptr = self.as_ptr();
        if self.is_process() {
            let p: Box<ProcessData> = unsafe { Box::from_raw(ptr.cast()) };
            drop(p);
        } else {
            let p: Box<Branch> = unsafe { Box::from_raw(ptr.cast()) };
            drop(p);
        }
    }
}

impl fmt::Debug for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ptr = self.as_ptr();
        if self.is_process() {
            let p: &ProcessData = unsafe { &mut *(ptr.cast()) };
            let p: Pin<&ProcessData> = unsafe { Pin::new_unchecked(p) };
            p.fmt(f)
        } else {
            let p: &Branch = unsafe { &mut *(ptr.cast()) };
            let p: Pin<&Branch> = unsafe { Pin::new_unchecked(p) };
            p.fmt(f)
        }
    }
}

/// Returns the depth at which `pid1` and `pid2` use difference branches.
fn diff_branch_depth(pid1: ProcessId, pid2: ProcessId) -> usize {
    debug_assert!(pid1 != pid2);
    ((pid1.0 ^ pid2.0) >> SKIP_BITS).trailing_zeros() as usize / LEVEL_SHIFT
}

/// Skip the bits up to `depth`.
const fn skip_bits(pid: ProcessId, depth: usize) -> usize {
    pid.0 >> ((depth * LEVEL_SHIFT) + SKIP_BITS)
}

#[cfg(test)]
mod tests {
    use std::cmp::max;
    use std::future::Future;
    use std::mem::align_of;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{self, Poll};

    use crate::process::{Process, ProcessId};
    use crate::spawn::options::Priority;

    use super::{
        diff_branch_depth, Branch, Inactive, Pointer, ProcessData, LEVEL_SHIFT, N_BRANCHES,
        POINTER_TAG_BITS, SKIP_BITS,
    };

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

    fn pow2(exp: usize) -> usize {
        2_usize.pow(exp as u32)
    }

    #[test]
    fn constants() {
        assert_eq!(pow2(LEVEL_SHIFT), N_BRANCHES);
    }

    #[test]
    fn size_assertions() {
        assert_eq!(size_of::<Pin<Box<ProcessData>>>(), size_of::<usize>());
        assert_eq!(size_of::<Pin<Box<Branch>>>(), size_of::<usize>());
        assert_eq!(size_of::<Pointer>(), size_of::<usize>());
        assert_eq!(size_of::<Branch>(), N_BRANCHES * size_of::<usize>());
    }

    #[test]
    fn process_data_alignment() {
        // Ensure that we don't skip any used bites and that the pointer tag
        // doesn't overwrite any pointer data.
        assert!(align_of::<ProcessData>() >= pow2(max(SKIP_BITS, POINTER_TAG_BITS)));
    }

    #[test]
    fn branch_alignment() {
        // Ensure that we don't skip any used bites and that the pointer tag
        // doesn't overwrite any pointer data.
        assert!(align_of::<Branch>() >= pow2(max(SKIP_BITS, POINTER_TAG_BITS)));
    }

    #[test]
    fn test_diff_branch_depth() {
        #[rustfmt::skip]
        let tests = &[
            (ProcessId(0b0001_00), ProcessId(0b0000_00), 0),
            (ProcessId(0b0000_00), ProcessId(0b0001_00), 0),
            (ProcessId(0b0010_00), ProcessId(0b0000_00), 0),
            (ProcessId(0b0000_00), ProcessId(0b0010_00), 0),
            (ProcessId(0b0100_00), ProcessId(0b0000_00), 0),
            (ProcessId(0b0000_00), ProcessId(0b0100_00), 0),
            (ProcessId(0b1000_00), ProcessId(0b0000_00), 0),
            (ProcessId(0b0000_00), ProcessId(0b1000_00), 0),
            (ProcessId(0b0001_0000_00), ProcessId(0b0000_0000_00), 1),
            (ProcessId(0b0000_0000_00), ProcessId(0b0001_0000_00), 1),
            (ProcessId(0b0010_0000_00), ProcessId(0b0000_0000_00), 1),
            (ProcessId(0b0000_0000_00), ProcessId(0b0010_0000_00), 1),
            (ProcessId(0b0100_0000_00), ProcessId(0b0000_0000_00), 1),
            (ProcessId(0b0000_0000_00), ProcessId(0b0100_0000_00), 1),
            (ProcessId(0b1000_0000_00), ProcessId(0b0000_0000_00), 1),
            (ProcessId(0b0000_0000_00), ProcessId(0b1000_0000_00), 1),
            (ProcessId(0b0001_0000_0000_00), ProcessId(0b0000_0000_0000_00), 2),
            (ProcessId(0b0000_0000_0000_00), ProcessId(0b0001_0000_0000_00), 2),
            (ProcessId(0b0010_0000_0000_00), ProcessId(0b0000_0000_0000_00), 2),
            (ProcessId(0b0000_0000_0000_00), ProcessId(0b0010_0000_0000_00), 2),
            (ProcessId(0b0100_0000_0000_00), ProcessId(0b0000_0000_0000_00), 2),
            (ProcessId(0b0000_0000_0000_00), ProcessId(0b0100_0000_0000_00), 2),
            (ProcessId(0b1000_0000_0000_00), ProcessId(0b0000_0000_0000_00), 2),
            (ProcessId(0b0000_0000_0000_00), ProcessId(0b1000_0000_0000_00), 2),
        ];

        for (pid1, pid2, expected) in tests.iter().copied() {
            let got = diff_branch_depth(pid1, pid2);
            assert_eq!(
                got, expected,
                "pid1: {:064b} ({}), pid2: {:064b} ({})",
                pid1.0, pid1, pid2.0, pid2
            );
        }
    }

    #[test]
    fn pointer_take_process() {
        // None.
        let mut ptr: Option<Pointer> = None;
        assert!(Pointer::take_process(&mut ptr).is_none());
        assert!(ptr.is_none());

        // Process -> Some(Ok(..)).
        let mut ptr: Option<Pointer> = Some(test_process().into());
        match Pointer::take_process(&mut ptr) {
            Some(Ok(_)) => {}
            _ => panic!("unexpected result"),
        }
        // Process is removed.
        assert!(ptr.is_none());

        // Branch -> Some(Err(..)).
        let mut ptr: Option<Pointer> = Some(Box::pin(Branch::empty()).into());
        match Pointer::take_process(&mut ptr) {
            Some(Err(_)) => {}
            _ => panic!("unexpected result"),
        }
        // Pointer unchanged.
        assert!(ptr.is_some());
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

        let dropped = Arc::new(AtomicUsize::new(0));

        let process = Box::pin(DropTest(dropped.clone()));
        let ptr: Pointer = Box::pin(ProcessData::new(Priority::default(), process)).into();

        assert_eq!(dropped.load(Ordering::Acquire), 0);
        drop(ptr);
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    fn add_process(queue: &mut Inactive) -> ProcessId {
        let process = test_process();
        let pid = process.as_ref().id();
        queue.add(process);
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
            let process = if let Some(p) = queue.remove(pid) {
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
            assert!(queue.remove(pid).is_none());
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
