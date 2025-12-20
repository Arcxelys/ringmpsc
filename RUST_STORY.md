# RUST_STORY.md - The Quest for Zero-Cost Abstractions

## ✅ MISSION ACCOMPLISHED (v2 - Even Better!)

Matched AND exceeded Zig's ring buffer throughput using Rust's zero-cost abstractions.

---

## Final Benchmark Results (AMD Ryzen 7 PRO 8840U - Zen 4)

```
┌─────────────────────────────────────────────────────────────┐
│ STACK RING (embedded buffer, zero heap indirection)        │
├─────────────┬───────────────┬───────────────┬──────────────┤
│ Config      │ Zig           │ Rust          │ vs Zig       │
├─────────────┼───────────────┼───────────────┼──────────────┤
│ 1P1C        │ ~0.46 B/s     │ ~0.55 B/s     │ +20% ✓✓      │
│ 2P2C        │ ~0.45 B/s     │ ~0.78 B/s     │ +73% ✓✓      │
│ 4P4C        │ ~0.75 B/s     │ ~1.35 B/s     │ +80% ✓✓      │
│ 6P6C        │ ~1.04 B/s     │ ~1.50 B/s     │ +44% ✓✓      │
│ 8P8C        │ ~1.02 B/s     │ ~1.38 B/s     │ +35% ✓✓      │
└─────────────┴───────────────┴───────────────┴──────────────┘
```

**Rust beats Zig on ALL configurations!**

---

## Optimizations That Worked

### 1. Smart CPU Pinning (A/B Tested)
**Finding**: Pinning helps for 1P1C but hurts for multi-producer scenarios.  
**Fix**: Enable pinning only for 1P1C configuration.  
**Impact**: +30% for 1P1C throughput.

```rust
// Pin only for 1P1C (A/B test showed improvement)
let use_pinning = num_pairs == 1;
if use_pinning {
    pin_to_cpu(cpu_id);
}
```

### 2. Removed Software Prefetch (A/B Tested)
**Finding**: Software prefetch hurts performance on AMD Zen 4!  
**Test Result**: +15% throughput WITHOUT prefetch.  
**Reason**: The hardware prefetcher handles sequential access patterns better.

```rust
// BEFORE: Software prefetch (hurts performance)
prefetch_write(self.buffer.as_ptr().add(next_idx) as *mut T);

// AFTER: Let hardware prefetcher do its job
// (no prefetch instruction - hardware handles it)
```

### 3. Batch Consumption (algorithmic)
**Problem**: Consumer called `peek()` + `advance()` per iteration = per-item atomic ops.  
**Fix**: Implemented `consume_batch()` which does single `head.store()` for N items.  
**Impact**: Amortizes atomic overhead across entire batch.

```rust
pub unsafe fn consume_batch<F>(&self, mut handler: F) -> usize
where F: FnMut(&T)
{
    let head = self.head.load(Ordering::Relaxed);
    let tail = self.tail.load(Ordering::Acquire);
    
    // Process all items
    let mut pos = head;
    while pos != tail {
        let idx = (pos as usize) & Self::MASK;
        handler(&*self.buffer[idx]);
        pos = pos.wrapping_add(1);
    }
    
    // Single atomic update for entire batch!
    self.head.store(pos, Ordering::Release);
    (tail - head) as usize
}
```

---

## What Was Tried (And Didn't Help)

### 1. Custom ThinArc (`raw_arc.rs`)
**Hypothesis**: `std::sync::Arc` has double indirection.  
**Result**: No improvement — Arc overhead wasn't the bottleneck.

### 2. Stack-Allocated Ring (`stack_ring.rs`) Alone
**Hypothesis**: Embedded buffer would eliminate pointer chase.  
**Result**: No improvement alone — needed the other fixes too.

### 3. Consumer-Side Prefetch (A/B Tested)
**Hypothesis**: Prefetch ahead in consume loop would help.  
**Result**: **-40% throughput!** Added overhead with no benefit.

---

## A/B Testing Methodology

Created specialized benchmarks to isolate each optimization:

1. **`bench_ab.rs`**: Compares pinned vs unpinned threads
2. **`bench_prefetch.rs`**: Compares with/without software prefetch

Key findings from A/B testing:
- Pinning: +30% for 1P1C, -40% for 4P4C
- No prefetch: +15% for 1P1C

---

## Project Structure

```
/home/a112/Documents/code/HFT/ringmpsc/
├── rust_impl/
│   ├── src/
│   │   ├── lib.rs            # Ring + Channel implementation
│   │   ├── atomics.rs        # x86_64 prefetch intrinsics
│   │   ├── stack_ring.rs     # Zero-indirection ring with consume_batch()
│   │   └── bin/
│   │       ├── bench.rs      # Main benchmark
│   │       ├── bench_ab.rs   # A/B test: pinning
│   │       └── bench_prefetch.rs  # A/B test: prefetch
│   └── Cargo.toml            # LTO=fat, codegen-units=1, opt-level=3
└── src/
    ├── channel.zig           # Zig reference implementation
    └── bench_final.zig       # Zig benchmark
```

---

## Run Commands

```bash
# Rust benchmark (use target-cpu=native for best results)
cd rust_impl && RUSTFLAGS="-C target-cpu=native" cargo run --release

# Zig benchmark  
zig build -Doptimize=ReleaseFast && ./zig-out/bin/bench

# A/B tests
RUSTFLAGS="-C target-cpu=native" cargo run --release --bin bench_ab
RUSTFLAGS="-C target-cpu=native" cargo run --release --bin bench_prefetch
```

---

## Key Lessons

1. **A/B test everything**: "Optimizations" can hurt performance (prefetch on Zen 4)
2. **Platform-specific tuning**: What works on Intel may not work on AMD
3. **Hardware prefetcher is smart**: Modern CPUs handle sequential patterns well
4. **Pinning is situational**: Great for single pairs, harmful for multi-core scaling
5. **Batch amortization wins**: One atomic op for N items >> N atomic ops
6. **Disassembly doesn't lie**: Always verify generated instructions match intent
7. **Zero-cost abstractions are real**: Rust matches C/Zig when you get the details right
