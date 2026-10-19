//! Discrete-event scheduling and the per-validator CPU model.

use std::{cmp::Ordering, collections::BinaryHeap};

/// Min-heap of events ordered by time, ties broken by insertion order.
#[derive(Debug)]
pub struct EventQueue<E> {
    heap: BinaryHeap<Entry<E>>,
    seq: u64,
}

#[derive(Debug)]
struct Entry<E> {
    time: u64,
    seq: u64,
    event: E,
}

impl<E> PartialEq for Entry<E> {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.seq == other.seq
    }
}

impl<E> Eq for Entry<E> {}

impl<E> PartialOrd for Entry<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E> Ord for Entry<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.time, other.seq).cmp(&(self.time, self.seq))
    }
}

impl<E> Default for EventQueue<E> {
    fn default() -> Self {
        Self { heap: BinaryHeap::new(), seq: 0 }
    }
}

impl<E> EventQueue<E> {
    /// Schedules `event` at `time`.
    pub fn push(&mut self, time: u64, event: E) {
        self.seq += 1;
        self.heap.push(Entry { time, seq: self.seq, event });
    }

    /// Removes the earliest event.
    pub fn pop(&mut self) -> Option<(u64, E)> {
        self.heap.pop().map(|e| (e.time, e.event))
    }
}

/// CPU of every validator: one serial execution lane plus a pool of cores for signatures and
/// sender recovery.
#[derive(Debug)]
pub struct Cpu {
    exec_free: Vec<u64>,
    sig_free: Vec<Vec<u64>>,
    /// Execution-lane busy time per validator inside the measurement window, in microseconds.
    pub exec_busy: Vec<f64>,
    /// Signature-pool busy time per validator inside the measurement window, in microseconds.
    pub sig_busy: Vec<f64>,
    window: (u64, u64),
}

impl Cpu {
    /// Creates idle CPUs. Busy time is only counted for work starting inside `window`.
    pub fn new(n: usize, sig_cores: usize, window: (u64, u64)) -> Self {
        Self {
            exec_free: vec![0; n],
            sig_free: vec![vec![0; sig_cores.max(1)]; n],
            exec_busy: vec![0.0; n],
            sig_busy: vec![0.0; n],
            window,
        }
    }

    /// Runs `cost_us` of serial work on the execution lane of `node` once `ready` has passed.
    /// Returns the completion time.
    pub fn exec(&mut self, node: usize, ready: u64, cost_us: f64) -> u64 {
        let start = self.exec_free[node].max(ready);
        let done = start + cost_us.ceil() as u64;
        self.exec_free[node] = done;
        if self.in_window(start) {
            self.exec_busy[node] += cost_us;
        }
        done
    }

    /// Runs `cost_us` on the earliest free signature core of `node`. Returns the completion time.
    pub fn sig(&mut self, node: usize, ready: u64, cost_us: f64) -> u64 {
        let cores = &mut self.sig_free[node];
        let (idx, free) =
            cores.iter().copied().enumerate().min_by_key(|(_, t)| *t).expect("at least one core");
        let start = free.max(ready);
        let done = start + cost_us.ceil() as u64;
        cores[idx] = done;
        if self.in_window(start) {
            self.sig_busy[node] += cost_us;
        }
        done
    }

    /// Spreads `cost_us` of parallel work (such as sender recovery of a block) over all
    /// signature cores of `node`. Returns the time the last share completes.
    pub fn parallel(&mut self, node: usize, ready: u64, cost_us: f64) -> u64 {
        let cores = self.sig_free[node].len();
        let share = cost_us / cores as f64;
        (0..cores).map(|_| self.sig(node, ready, share)).max().unwrap_or(ready)
    }

    const fn in_window(&self, t: u64) -> bool {
        t >= self.window.0 && t < self.window.1
    }
}
