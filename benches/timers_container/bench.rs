#![feature(binary_heap_retain, result_into_ok_or_err, map_first_last)]

use std::cmp::max;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use rand::{RngCore, SeedableRng};
use rand_xoshiro::Xoshiro128PlusPlus;

criterion_main!(containers);
criterion_group!(containers, add, remove_next, remove, remove_already_removed);

pub fn add(c: &mut Criterion) {
    let mut group = c.benchmark_group("Adding timer");
    binary_heap::add_timer(&mut group);
    btreemap::add_timer(&mut group);
    sorted_vec::add_timer(&mut group);
    group.finish();
}

pub fn remove_next(c: &mut Criterion) {
    let mut group = c.benchmark_group("Removing next timer");
    binary_heap::remove_next(&mut group);
    btreemap::remove_next(&mut group);
    sorted_vec::remove_next(&mut group);
    group.finish();
}

pub fn remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("Removing timer");
    binary_heap::remove(&mut group);
    btreemap::remove(&mut group);
    sorted_vec::remove(&mut group);
    group.finish();
}

pub fn remove_already_removed(c: &mut Criterion) {
    let mut group = c.benchmark_group("Removing timer (already removed)");
    binary_heap::remove_already_removed(&mut group);
    btreemap::remove_already_removed(&mut group);
    sorted_vec::remove_already_removed(&mut group);
    group.finish();
}

mod binary_heap {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    use criterion::measurement::Measurement;
    use criterion::{BatchSize, BenchmarkGroup};

    use crate::{new_timers, remove_timers, start_timers, Timer, START_SIZE};

    pub fn add_timer<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BinaryHeap", |b| {
            let initial_heap = create_heap();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_heap.clone(),
                |mut heap| {
                    let timer = timers.next().unwrap();
                    heap.push(Reverse(timer));
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_next<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BinaryHeap", |b| {
            let initial_heap = create_heap();
            b.iter_batched(
                || initial_heap.clone(),
                |mut heap| heap.pop(),
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BinaryHeap", |b| {
            let initial_heap = create_heap();
            let mut timers = remove_timers();
            b.iter_batched(
                || initial_heap.clone(),
                |mut heap| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut heap, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_already_removed<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BinaryHeap", |b| {
            let initial_heap = create_heap();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_heap.clone(),
                |mut heap| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut heap, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    fn remove_timer(heap: &mut BinaryHeap<Reverse<Timer>>, timer: Timer) {
        let mut found = false;
        heap.retain(|d| {
            found
                || if d.0.pid == timer.pid && d.0.deadline == timer.deadline {
                    found = true;
                    false
                } else {
                    true
                }
        });
    }

    fn create_heap() -> BinaryHeap<Reverse<Timer>> {
        let mut heap = BinaryHeap::new();
        for timer in start_timers().take(START_SIZE) {
            heap.push(Reverse(timer));
        }
        heap
    }
}

mod btreemap {
    use std::collections::btree_map::{BTreeMap, Entry};
    use std::time::Instant;

    use criterion::measurement::Measurement;
    use criterion::{BatchSize, BenchmarkGroup};

    use crate::{new_timers, remove_timers, start_timers, ProcessId, Timer, START_SIZE};

    pub fn add_timer<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BTreeMap", |b| {
            let initial_map = create_map();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_map.clone(),
                |mut map| {
                    let timer = timers.next().unwrap();
                    map.insert(timer.deadline, timer.pid);
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_next<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BTreeMap", |b| {
            let initial_map = create_map();
            b.iter_batched(
                || initial_map.clone(),
                |mut map| map.pop_first(),
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BTreeMap", |b| {
            let initial_map = create_map();
            let mut timers = remove_timers();
            b.iter_batched(
                || initial_map.clone(),
                |mut map| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut map, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_already_removed<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("BTreeMap", |b| {
            let initial_map = create_map();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_map.clone(),
                |mut map| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut map, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    fn remove_timer(map: &mut BTreeMap<Instant, ProcessId>, timer: Timer) {
        match map.entry(timer.deadline) {
            Entry::Vacant(_) => {} // Already removed.
            Entry::Occupied(entry) => {
                if *entry.get() == timer.pid {
                    let _ = entry.remove();
                }
                // Different process id, don't remove it.
            }
        }
    }

    fn create_map() -> BTreeMap<Instant, ProcessId> {
        let mut map = BTreeMap::new();
        for timer in start_timers().take(START_SIZE) {
            let r = map.insert(timer.deadline, timer.pid);
            assert!(r.is_none(), "timer: {timer:?}");
        }
        map
    }
}

mod sorted_vec {
    use std::cmp::Reverse;

    use criterion::measurement::Measurement;
    use criterion::{BatchSize, BenchmarkGroup};

    use crate::{new_timers, remove_timers, start_timers, Timer, START_SIZE};

    pub fn add_timer<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("sorted Vec", |b| {
            let initial_vec = create_vec();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_vec.clone(),
                |mut vec| {
                    let timer = Reverse(timers.next().unwrap());
                    let idx = vec.binary_search(&timer).into_ok_or_err();
                    vec.insert(idx, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_next<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("sorted Vec", |b| {
            let initial_vec = create_vec();
            b.iter_batched(
                || initial_vec.clone(),
                |mut vec| vec.pop(),
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("sorted Vec", |b| {
            let initial_vec = create_vec();
            let mut timers = remove_timers();
            b.iter_batched(
                || initial_vec.clone(),
                |mut vec| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut vec, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    pub fn remove_already_removed<M: Measurement>(group: &mut BenchmarkGroup<M>) {
        group.bench_function("sorted Vec", |b| {
            let initial_vec = create_vec();
            let mut timers = new_timers();
            b.iter_batched(
                || initial_vec.clone(),
                |mut vec| {
                    let timer = timers.next().unwrap();
                    remove_timer(&mut vec, timer);
                },
                BatchSize::SmallInput,
            );
        });
    }

    fn remove_timer(vec: &mut Vec<Reverse<Timer>>, timer: Timer) {
        let timer = Reverse(timer);
        if let Ok(idx) = vec.binary_search(&timer) {
            let _ = vec.remove(idx);
        }
    }

    fn create_vec() -> Vec<Reverse<Timer>> {
        let mut vec = Vec::with_capacity(START_SIZE);
        for timer in start_timers().take(START_SIZE) {
            vec.push(Reverse(timer));
        }
        vec.sort();
        vec
    }
}

/// Number of starting elements in the container.
const START_SIZE: usize = 300;

type ProcessId = u64; // NOTE: Is really a `usize`.

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
struct Timer {
    pid: ProcessId,
    deadline: Instant,
}

/// Returns a generator for starting timers.
fn start_timers() -> GenTimers {
    #[allow(clippy::unreadable_literal)]
    const SEED: [u8; 16] = 173328903770940342687532334189206051087_u128.to_be_bytes();
    GenTimers {
        prng: Xoshiro128PlusPlus::from_seed(SEED),
        epoch: Instant::now(),
    }
}

/// Returns a generator for timers to remove.
fn remove_timers() -> GenTimers {
    start_timers()
}

fn new_timers() -> GenTimers {
    #[allow(clippy::unreadable_literal)]
    const SEED: [u8; 16] = 113816226723235353907830994955339774107_u128.to_be_bytes();
    GenTimers {
        prng: Xoshiro128PlusPlus::from_seed(SEED),
        epoch: Instant::now(),
    }
}

/// Timer generator.
struct GenTimers {
    /// Pseudo-random number generator.
    prng: Xoshiro128PlusPlus,
    epoch: Instant,
}

impl Iterator for GenTimers {
    type Item = Timer;

    fn next(&mut self) -> Option<Self::Item> {
        let pid = self.prng.next_u64();
        let mut add = [0; 2];
        self.prng.fill_bytes(&mut add);
        // The containers use `Instant` as key so they have to be unique.
        let add = max(u16::from_ne_bytes(add), 1);
        let deadline = self.epoch + Duration::from_nanos(add.into());
        self.epoch = deadline;
        Some(Timer { pid, deadline })
    }
}
