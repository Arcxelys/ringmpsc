//! A/B Test Benchmark for RingMPSC optimizations
//! Tests different configurations: prefetch vs no-prefetch, pinning vs no-pinning

use rust_impl::stack_ring::StackRing;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

const MSG: u64 = 100_000_000; // 100M messages per producer
const RING_SIZE: usize = 1 << 16; // 64K slots
const WARMUP_RUNS: usize = 2;
const BENCH_RUNS: usize = 5;

fn main() {
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("║                   RINGMPSC - A/B TEST BENCHMARK              ║");
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        "Config: {}M msgs/producer, ring={}K slots",
        MSG / 1_000_000,
        RING_SIZE >> 10
    );
    println!(
        "Warmup: {} runs, Benchmark: {} runs (median reported)\n",
        WARMUP_RUNS, BENCH_RUNS
    );

    // Test configurations
    let configs = [
        ("1P1C No Pin", 1, false),
        ("1P1C Pinned", 1, true),
        ("2P2C No Pin", 2, false),
        ("2P2C Pinned", 2, true),
        ("4P4C No Pin", 4, false),
        ("4P4C Pinned", 4, true),
    ];

    println!("┌──────────────┬───────────────┬──────────────┬─────────────┐");
    println!("│ Config       │ Throughput    │ Std Dev      │ Improvement │");
    println!("├──────────────┼───────────────┼──────────────┼─────────────┤");

    let mut last_rate = 0.0f64;
    for (name, pairs, pinned) in configs {
        let rates = run_benchmark(pairs, pinned);
        let (median, stddev) = stats(&rates);

        let improvement = if last_rate > 0.0 && pairs == configs[configs.len() - 2].1 {
            format!("{:+.1}%", (median / last_rate - 1.0) * 100.0)
        } else {
            "-".to_string()
        };

        println!(
            "│ {:12} │ {:>8.3} B/s  │ ±{:6.3} B/s  │ {:>10}  │",
            name, median, stddev, improvement
        );

        if !pinned {
            last_rate = median;
        }
    }

    println!("└──────────────┴───────────────┴──────────────┴─────────────┘\n");
}

fn run_benchmark(num_pairs: usize, pinned: bool) -> Vec<f64> {
    // Warmup
    for _ in 0..WARMUP_RUNS {
        let _ = run_test(num_pairs, pinned);
    }

    // Actual runs
    (0..BENCH_RUNS)
        .map(|_| run_test(num_pairs, pinned))
        .collect()
}

fn run_test(num_pairs: usize, pinned: bool) -> f64 {
    let rings: Vec<&'static StackRing<u32, RING_SIZE>> = (0..num_pairs)
        .map(|_| &*Box::leak(Box::new(StackRing::new())))
        .collect();

    let counts: Arc<Vec<AtomicU64>> = Arc::new((0..num_pairs).map(|_| AtomicU64::new(0)).collect());

    let t0 = Instant::now();

    // Start consumers (pinned to CPUs num_pairs..2*num_pairs)
    let mut consumer_threads = Vec::with_capacity(num_pairs);
    for i in 0..num_pairs {
        let ring = rings[i];
        let counts_clone = counts.clone();
        let cpu_id = num_pairs + i;
        consumer_threads.push(thread::spawn(move || {
            if pinned {
                pin_to_cpu(cpu_id);
            }
            let mut count = 0u64;
            loop {
                unsafe {
                    let n = ring.consume_batch(|_| {});
                    if n > 0 {
                        count += n as u64;
                    } else if ring.is_closed() && ring.is_empty() {
                        break;
                    } else {
                        std::hint::spin_loop();
                    }
                }
            }
            counts_clone[i].store(count, Ordering::Release);
        }));
    }

    // Start producers (pinned to CPUs 0..num_pairs)
    let mut producer_threads = Vec::with_capacity(num_pairs);
    for i in 0..num_pairs {
        let ring = rings[i];
        producer_threads.push(thread::spawn(move || {
            if pinned {
                pin_to_cpu(i);
            }
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
        }));
    }

    for t in producer_threads {
        t.join().unwrap();
    }
    for t in consumer_threads {
        t.join().unwrap();
    }

    let elapsed = t0.elapsed();
    let ns = elapsed.as_nanos() as f64;
    let total: u64 = counts.iter().map(|c| c.load(Ordering::Acquire)).sum();

    total as f64 / ns
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
    let stddev = variance.sqrt();

    (median, stddev)
}
