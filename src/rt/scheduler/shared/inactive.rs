use std::fmt;
use std::mem::{forget, replace};
use std::pin::Pin;
use std::ptr::NonNull;

use crate::rt::scheduler::shared::ProcessData;
use crate::rt::scheduler::ProcessId;

/// Number of branches per level of the tree, must be a power of 2.
// TODO: benchmark a size of 2, only using 2 bits at a time. This would increase
// the maximum depth of the tree to ~64 (from ~32).
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
        self.length == 0
    }

    /// Add a `process`.
    pub(super) fn add(&mut self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        // Ensure `SKIP_BITS` is correct.
        debug_assert!(pid.0 & SKIP_MASK == 0);
        self.root.add(process, pid.0 >> SKIP_BITS, 0);
        self.length += 1;
    }

    /// Removes the process with id `pid`, if any.
    pub(super) fn remove(&mut self, pid: ProcessId) -> Option<Pin<Box<ProcessData>>> {
        self.root.remove(pid, pid.0 >> SKIP_BITS).map(|process| {
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

    fn add(&mut self, process: Pin<Box<ProcessData>>, w_pid: usize, depth: usize) {
        match Pointer::take_process(&mut self.branches[w_pid & LEVEL_MASK]) {
            Some(Ok(other_process)) => {
                // This branch ends in another process.
                self.add_both(process, other_process, w_pid, depth);
            }
            Some(Err(mut branch)) => branch.add(process, w_pid >> LEVEL_SHIFT, depth + 1),
            None => self.branches[w_pid & LEVEL_MASK] = Some(process.into()),
        }
    }

    /// Add two processes at a time.
    ///
    /// Used when a process is added but another process is encountered. Instead
    /// of adding one process (by creating a new level) and then add the other,
    /// possibly creating another conflict. We first create enough levels to not
    /// have conflicting branches and then add both processes.
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
        let req_depth = diff_branch_depth(pid1, pid2, depth);
        // Add the two processes.
        let w_pid1 = skip_bits(pid1, depth + req_depth);
        branch.branches[w_pid1 & LEVEL_MASK] = Some(process1.into());
        let w_pid2 = skip_bits(pid2, depth + req_depth);
        branch.branches[w_pid2 & LEVEL_MASK] = Some(process2.into());
        debug_assert!(w_pid1 & LEVEL_MASK != w_pid2 & LEVEL_MASK);
        // Build up to route to the branch.
        for depth in 1..req_depth {
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

/// Tags used for the `Pointer`.
const PROCESS_TAG: usize = 0b1;
const BRANCH_TAG: usize = 0b0;

impl Pointer {
    /// Attempts to take a process pointer from `this`.
    ///
    /// Returns:
    /// * `None` if `this` is `None`.
    /// * `Some(Ok(..))` if the pointer is `Some` and points to a process.
    /// * `Some(Err(..))` if the pointer is `Some` and points to a branch.
    fn take_process<'a>(
        this: &'a mut Option<Pointer>,
    ) -> Option<Result<Pin<Box<ProcessData>>, Pin<&'a mut Branch>>> {
        match this {
            Some(pointer) if pointer.is_process() => {
                let p = unsafe { Box::from_raw(pointer.as_ptr() as *mut _) };
                // We just read the pointer so now we have to forget it.
                forget(this.take());
                Some(Ok(Pin::new(p)))
            }
            Some(pointer) => {
                debug_assert!(!pointer.is_process());
                let p: &mut Branch = unsafe { &mut *(pointer.as_ptr() as *mut _) };
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
        if self.is_process() {
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
    use std::time::Duration;

    use crate::rt::process::{Process, ProcessId, ProcessResult};
    use crate::rt::scheduler::Priority;
    use crate::rt::RuntimeRef;

    use super::{Branch, Inactive, Pointer, ProcessData};

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
        fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
            unimplemented!()
        }
    }

    fn test_process() -> Pin<Box<ProcessData>> {
        Box::pin(ProcessData {
            priority: Priority::default(),
            fair_runtime: Duration::from_secs(0),
            process: Box::pin(TestProcess),
        })
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
    }

    #[test]
    fn branch_alignment() {
        // Ensure that the pointer tag doesn't overwrite any pointer data.
        assert!(align_of::<Branch>() >= 2);
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
                let _ = self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        impl Process for DropTest {
            fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
                unimplemented!()
            }
        }

        let dropped = Arc::new(AtomicUsize::new(0));

        let process = Box::pin(DropTest(dropped.clone()));
        let ptr: Pointer = Box::pin(ProcessData {
            priority: Priority::default(),
            fair_runtime: Duration::from_secs(0),
            process,
        })
        .into();

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
