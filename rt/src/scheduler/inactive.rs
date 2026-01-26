use std::cmp::max;
use std::fmt;
use std::mem::replace;
use std::pin::Pin;

use crate::scheduler::{Process, ProcessId};
use crate::util::TaggedPointer;
use crate::worker::SYSTEM_ACTORS;

/// Number of bits to shift per level.
const LEVEL_SHIFT: usize = 4;
/// Number of branches per level of the tree, must be a power of 2.
const N_BRANCHES: usize = 1 << LEVEL_SHIFT; // 16
/// Number of bits to mask per level.
const LEVEL_MASK: usize = (1 << LEVEL_SHIFT) - 1;
/// For alignment reasons the two least significant bits of a boxed `Process`
/// are always 0, so we can safely skip them. Also see `ok_pid` and alignment
/// tests below.
const SKIP_BITS: usize = 2;
const SKIP_MASK: usize = (1 << SKIP_BITS) - 1;

/// Returns `false` if `pid`'s `SKIP_BITS` aren't valid.
fn ok_pid(pid: ProcessId) -> bool {
    pid.0 & max(SKIP_MASK, TaggedPointer::TAG_BITS) == 0
}

/// Inactive processes.
///
/// Implemented as a tree with four pointers on each level. A pointer can point
/// to a `Branch`, which again contains four pointers, or point to `Process`.
///
/// Indexing into the structure is done using the `ProcessId` of the process,
/// however the pointer itself points to `Process`.
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
pub(crate) struct Inactive<S> {
    root: Branch<S>,
    length: usize,
}

impl<S> Inactive<S> {
    /// Create an empty `Inactive` tree.
    pub(crate) const fn empty() -> Inactive<S> {
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
    pub(crate) fn add(&mut self, process: Pin<Box<Process<S>>>) {
        let pid = process.id();
        debug_assert!(ok_pid(pid));
        self.root.add(process, pid.0 >> SKIP_BITS, 0);
        self.length += 1;
    }

    /// Removes the process with id `pid`, if any.
    pub(crate) fn remove(&mut self, pid: ProcessId) -> Option<Pin<Box<Process<S>>>> {
        debug_assert!(ok_pid(pid));
        self.root
            .remove(pid, pid.0 >> SKIP_BITS)
            .inspect(|process| {
                debug_assert_eq!(process.id(), pid);
                self.length -= 1;
            })
    }
}

struct Branch<S> {
    branches: [Option<TaggedPointer<Process<S>, Branch<S>>>; N_BRANCHES],
}

impl<S: fmt::Debug> fmt::Debug for Branch<S> {
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

impl<S> Branch<S> {
    const NONE: Option<TaggedPointer<Process<S>, Branch<S>>> = None;

    /// Create an empty `Branch`.
    const fn empty() -> Branch<S> {
        Branch {
            branches: [Branch::NONE; N_BRANCHES],
        }
    }

    fn add(&mut self, process: Pin<Box<Process<S>>>, w_pid: usize, depth: usize) {
        match TaggedPointer::take_left(&mut self.branches[w_pid & LEVEL_MASK]) {
            Some(Ok(other_process)) => self.add_both(process, other_process, w_pid, depth),
            Some(Err(mut branch)) => branch.add(process, w_pid >> LEVEL_SHIFT, depth + 1),
            None => self.branches[w_pid & LEVEL_MASK] = Some(TaggedPointer::left(process)),
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
        process1: Pin<Box<Process<S>>>,
        process2: Pin<Box<Process<S>>>,
        w_pid: usize,
        depth: usize,
    ) {
        debug_assert!(self.branches[w_pid & LEVEL_MASK].is_none());
        let pid1 = process1.id();
        let pid2 = process2.id();

        // Build the part of the branch in reverse, starting at the lowest
        // branch.
        let mut branch = Box::pin(Branch::empty());
        // Required depth to go were the pointers are in different slots.
        let req_depth = diff_branch_depth(pid1, pid2);
        debug_assert!(req_depth >= depth);
        // Add the two processes.
        let w_pid1 = skip_bits(pid1, req_depth);
        branch.branches[w_pid1 & LEVEL_MASK] = Some(TaggedPointer::left(process1));
        let w_pid2 = skip_bits(pid2, req_depth);
        branch.branches[w_pid2 & LEVEL_MASK] = Some(TaggedPointer::left(process2));
        debug_assert!(w_pid1 & LEVEL_MASK != w_pid2 & LEVEL_MASK);
        // Build up to route to the branch.
        for depth in depth + 1..req_depth {
            let index = (w_pid >> ((req_depth - depth) * LEVEL_SHIFT)) & LEVEL_MASK;
            let next_branch = replace(&mut branch, Box::pin(Branch::empty()));
            branch.branches[index] = Some(TaggedPointer::right(next_branch));
        }
        self.branches[w_pid & LEVEL_MASK] = Some(TaggedPointer::right(branch));
    }

    fn remove(&mut self, pid: ProcessId, w_pid: usize) -> Option<Pin<Box<Process<S>>>> {
        let node = &mut self.branches[w_pid & LEVEL_MASK];
        match TaggedPointer::take_left(node) {
            Some(Ok(process)) if process.id() == pid => Some(process),
            Some(Ok(process)) => {
                // Wrong process so put it back.
                *node = Some(TaggedPointer::left(process));
                None
            }
            Some(Err(mut next_branch)) => next_branch.remove(pid, w_pid >> LEVEL_SHIFT),
            None => None,
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
    use std::pin::Pin;
    use std::task::{self, Poll};

    use crate::scheduler::{Cfs, Process, ProcessId, process};
    use crate::spawn::options::Priority;

    use super::{Branch, Inactive, LEVEL_SHIFT, N_BRANCHES, diff_branch_depth};

    struct TestProcess;

    impl process::Run for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }

        fn run(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    fn test_process() -> Pin<Box<Process<Cfs>>> {
        Box::pin(Process::<Cfs>::new(
            Priority::default(),
            Box::pin(TestProcess),
        ))
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
        assert_eq!(size_of::<Pin<Box<Process<Cfs>>>>(), size_of::<usize>());
        assert_eq!(size_of::<Pin<Box<Branch<Cfs>>>>(), size_of::<usize>());
        assert_eq!(size_of::<Branch<Cfs>>(), N_BRANCHES * size_of::<usize>());
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

    fn add_process(queue: &mut Inactive<Cfs>) -> ProcessId {
        let process = test_process();
        let pid = process.id();
        queue.add(process);
        pid
    }

    fn test(remove_order: Vec<usize>) {
        let mut queue = Inactive::<Cfs>::empty();
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
            assert_eq!(process.id(), pid);
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
