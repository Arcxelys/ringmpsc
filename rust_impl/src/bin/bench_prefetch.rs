//! Prefetch A/B Test - comparing with and without prefetch instructions
//! for 1P1C configuration

use rust_impl::stack_ring::StackRing;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

const MSG: u64 = 100_000_000;
const RING_SIZE: usize = 1 << 16;
const RUNS: usize = 5;

// A version without prefetch for testing
#[repr(C)]
#[repr(align(128))]
pub struct NoPrefetchRing<T, const N: usize> {
    tail: AtomicU64,
    cached_head: UnsafeCell<u64>,
    head: AtomicU64,
    _pad: [u8; 120],
    cached_tail: UnsafeCell<u64>,
    closed: AtomicBool,
    buffer: [UnsafeCell<MaybeUninit<T>>; N],
}

unsafe impl<T: Send, const N: usize> Send for NoPrefetchRing<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for NoPrefetchRing<T, N> {}

impl<T, const N: usize> NoPrefetchRing<T, N> {
    const MASK: usize = N - 1;

    pub const fn new() -> Self {
        assert!(N > 0 && (N & (N - 1)) == 0, "N must be a power of 2");
        Self {
            tail: AtomicU64::new(0),
            cached_head: UnsafeCell::new(0),
            head: AtomicU64::new(0),
            _pad: [0; 120],
            cached_tail: UnsafeCell::new(0),
            closed: AtomicBool::new(false),
            buffer: unsafe { MaybeUninit::uninit().assume_init() },
        }
    }

    #[inline(always)]
    pub unsafe fn reserve(&self, n: usize) -> Option<(*mut T, usize)> {
        let tail = self.tail.load(Ordering::Relaxed);
        let cached_head_ptr = self.cached_head.get();
        let mut head = *cached_head_ptr;

        let used = tail.wrapping_sub(head);
        let mut free = (N as u64).wrapping_sub(used);

        if free < (n as u64) {
            head = self.head.load(Ordering::Acquire);
            *cached_head_ptr = head;
            let used = tail.wrapping_sub(head);
            free = (N as u64).wrapping_sub(used);
            if free < (n as u64) {
                return None;
            }
        }

        let idx = (tail as usize) & Self::MASK;
        let contiguous = n.min(N - idx);
        // NO PREFETCH HERE!
        let ptr = (*self.buffer.as_ptr().add(idx)).get() as *mut T;
        Some((ptr, contiguous))
    }

    #[inline(always)]
    pub fn commit(&self, n: usize) {
        let tail = self.tail.load(Ordering::Relaxed);
        self.tail
            .store(tail.wrapping_add(n as u64), Ordering::Release);
    }

    #[inline(always)]
    pub unsafe fn consume_batch<F>(&self, mut handler: F) -> usize
    where
        F: FnMut(&T),
    {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        let avail = tail.wrapping_sub(head);
        if avail == 0 {
            return 0;
        }

        let mut pos = head;
        while pos != tail {
            let idx = (pos as usize) & Self::MASK;
            let ptr = (*self.buffer.as_ptr().add(idx)).get() as *const T;
            handler(&*ptr);
            pos = pos.wrapping_add(1);
        }

        self.head.store(pos, Ordering::Release);
        *self.cached_tail.get() = tail;
        avail as usize
    }

    #[inline(always)]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.tail.load(Ordering::Relaxed) == self.head.load(Ordering::Relaxed)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

fn main() {
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("║             RINGMPSC - PREFETCH A/B TEST (1P1C)              ║");
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        "Config: {}M msgs/producer, ring={}K slots, {} runs\n",
        MSG / 1_000_000,
        RING_SIZE >> 10,
        RUNS
    );

    println!("┌────────────────────┬──────────────┬──────────────┐");
    println!("│ Config             │ Throughput   │ Std Dev      │");
    println!("├────────────────────┼──────────────┼──────────────┤");

    // Test with prefetch (original StackRing)
    let with_prefetch: Vec<f64> = (0..RUNS).map(|_| run_with_prefetch()).collect();
    let (med1, std1) = stats(&with_prefetch);
    println!(
        "│ With Prefetch      │ {:>7.3} B/s  │ ±{:5.3} B/s   │",
        med1, std1
    );

    // Test without prefetch
    let without_prefetch: Vec<f64> = (0..RUNS).map(|_| run_without_prefetch()).collect();
    let (med2, std2) = stats(&without_prefetch);
    println!(
        "│ Without Prefetch   │ {:>7.3} B/s  │ ±{:5.3} B/s   │",
        med2, std2
    );

    println!("└────────────────────┴──────────────┴──────────────┘");

    let delta = (med2 / med1 - 1.0) * 100.0;
    println!("\nDifference: {:+.1}% (without prefetch vs with)", delta);
}

fn run_with_prefetch() -> f64 {
    let ring: &'static StackRing<u32, RING_SIZE> = Box::leak(Box::new(StackRing::new()));
    run_test_generic(ring)
}

fn run_without_prefetch() -> f64 {
    let ring: &'static NoPrefetchRing<u32, RING_SIZE> = Box::leak(Box::new(NoPrefetchRing::new()));
    run_test_generic(ring)
}

trait RingOps<T> {
    unsafe fn reserve(&self, n: usize) -> Option<(*mut T, usize)>;
    fn commit(&self, n: usize);
    unsafe fn consume_batch<F: FnMut(&T)>(&self, handler: F) -> usize;
    fn is_closed(&self) -> bool;
    fn is_empty(&self) -> bool;
    fn close(&self);
}

impl<const N: usize> RingOps<u32> for StackRing<u32, N> {
    unsafe fn reserve(&self, n: usize) -> Option<(*mut u32, usize)> {
        StackRing::reserve(self, n)
    }
    fn commit(&self, n: usize) {
        StackRing::commit(self, n)
    }
    unsafe fn consume_batch<F: FnMut(&u32)>(&self, handler: F) -> usize {
        StackRing::consume_batch(self, handler)
    }
    fn is_closed(&self) -> bool {
        StackRing::is_closed(self)
    }
    fn is_empty(&self) -> bool {
        StackRing::is_empty(self)
    }
    fn close(&self) {
        StackRing::close(self)
    }
}

impl<const N: usize> RingOps<u32> for NoPrefetchRing<u32, N> {
    unsafe fn reserve(&self, n: usize) -> Option<(*mut u32, usize)> {
        NoPrefetchRing::reserve(self, n)
    }
    fn commit(&self, n: usize) {
        NoPrefetchRing::commit(self, n)
    }
    unsafe fn consume_batch<F: FnMut(&u32)>(&self, handler: F) -> usize {
        NoPrefetchRing::consume_batch(self, handler)
    }
    fn is_closed(&self) -> bool {
        NoPrefetchRing::is_closed(self)
    }
    fn is_empty(&self) -> bool {
        NoPrefetchRing::is_empty(self)
    }
    fn close(&self) {
        NoPrefetchRing::close(self)
    }
}

fn run_test_generic(ring: &'static (impl RingOps<u32> + Sync)) -> f64 {
    let count = Arc::new(AtomicU64::new(0));
    let count_clone = count.clone();

    let t0 = Instant::now();

    // Consumer (pinned to CPU 1)
    let consumer = thread::spawn(move || {
        pin_to_cpu(1);
        let mut c = 0u64;
        loop {
            unsafe {
                let n = ring.consume_batch(|_| {});
                if n > 0 {
                    c += n as u64;
                } else if ring.is_closed() && ring.is_empty() {
                    break;
                } else {
                    std::hint::spin_loop();
                }
            }
        }
        count_clone.store(c, Ordering::Release);
    });

    // Producer (pinned to CPU 0)
    let producer = thread::spawn(move || {
        pin_to_cpu(0);
        let mut sent = 0u64;
        while sent < MSG {
            unsafe {
                if let Some((ptr, len)) = ring.reserve(1) {
                    *ptr = sent as u32;
                    ring.commit(len);
                    sent += len as u64;
                } else {
                    std::hint::spin_loop();
                }
            }
        }
        ring.close();
    });

    producer.join().unwrap();
    consumer.join().unwrap();

    let ns = t0.elapsed().as_nanos() as f64;
    let total = count.load(Ordering::Acquire) as f64;
    total / ns
}

fn pin_to_cpu(cpu_id: usize) {
    if let Some(core_ids) = core_affinity::get_core_ids() {
        if cpu_id < core_ids.len() {
            core_affinity::set_for_current(core_ids[cpu_id]);
        }
    }
}

fn stats(rates: &[f64]) -> (f64, f64) {
    let mut sorted = rates.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if sorted.len() % 2 == 0 {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };
    let mean: f64 = rates.iter().sum::<f64>() / rates.len() as f64;
    let variance: f64 = rates.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rates.len() as f64;
    (median, variance.sqrt())
}
