/*
 * RingMPSC Benchmark - Matched methodology with Zig
 *
 * Compile: gcc -O3 -march=native -pthread bench.c -o bench
 * Run:     ./bench
 */

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <time.h>
#include <unistd.h>
#include <sched.h>

#include "ringmpsc.h"

#define MESSAGES_PER_PRODUCER 500000000ULL  /* 500M messages per producer */
#define BATCH_SIZE 32768                     /* Batch size for amortizing atomic ops */
#define NUM_CPUS 16                          /* Adjust for your system */

typedef struct {
    ring_t *ring;
    size_t messages_sent;
    int cpu_id;
} producer_args_t;

typedef struct {
    ring_t *ring;
    size_t messages_received;
    int cpu_id;
} consumer_args_t;

static uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

#ifdef __linux__
static void pin_to_cpu(int cpu) {
    int actual = cpu % NUM_CPUS;
    cpu_set_t cpuset;
    CPU_ZERO(&cpuset);
    CPU_SET(actual, &cpuset);
    pthread_setaffinity_np(pthread_self(), sizeof(cpuset), &cpuset);
}
#else
static void pin_to_cpu(int cpu) { (void)cpu; }
#endif

static inline void spin_hint(void) {
#if defined(__x86_64__) || defined(__i386__)
    __asm__ volatile("pause" ::: "memory");
#elif defined(__aarch64__)
    __asm__ volatile("yield" ::: "memory");
#else
    /* Fallback: empty */
#endif
}

static void *producer_thread(void *arg) {
    producer_args_t *args = (producer_args_t *)arg;
    ring_t *ring = args->ring;

    pin_to_cpu(args->cpu_id);

    uint64_t sent = 0;
    while (sent < MESSAGES_PER_PRODUCER) {
        size_t want = BATCH_SIZE;
        if (want > MESSAGES_PER_PRODUCER - sent) {
            want = (size_t)(MESSAGES_PER_PRODUCER - sent);
        }

        size_t contiguous;
        uint64_t *slots = ring_reserve_n(ring, want, &contiguous);
        if (slots) {
            /* Write pattern - unrolled 4-way */
            size_t i = 0;
            while (i + 4 <= contiguous) {
                slots[i]     = sent + i;
                slots[i + 1] = sent + i + 1;
                slots[i + 2] = sent + i + 2;
                slots[i + 3] = sent + i + 3;
                i += 4;
            }
            while (i < contiguous) {
                slots[i] = sent + i;
                i++;
            }
            ring_commit(ring, contiguous);
            sent += contiguous;
        } else {
            spin_hint();
        }
    }

    args->messages_sent = (size_t)sent;
    return NULL;
}

/* Noop handler for batch consumption - compiler optimizes away */
static void noop_handler(const uint64_t *item, void *ctx) {
    (void)item;
    (void)ctx;
}

static void *consumer_thread(void *arg) {
    consumer_args_t *args = (consumer_args_t *)arg;
    ring_t *ring = args->ring;

    pin_to_cpu(args->cpu_id);

    size_t count = 0;
    while (1) {
        size_t consumed = ring_consume_batch(ring, noop_handler, NULL);
        count += consumed;
        if (consumed == 0) {
            if (ring_is_closed(ring) && ring_is_empty(ring)) break;
            spin_hint();
        }
    }

    args->messages_received = count;
    return NULL;
}

static void run_benchmark(int num_pairs) {
    /* Allocate channel on heap */
    channel_t *channel = aligned_alloc(CACHE_LINE, sizeof(channel_t));
    if (!channel) {
        fprintf(stderr, "Failed to allocate channel\n");
        exit(1);
    }
    channel_init(channel);

    producer_args_t *producer_args = calloc(num_pairs, sizeof(producer_args_t));
    consumer_args_t *consumer_args = calloc(num_pairs, sizeof(consumer_args_t));
    pthread_t *producer_threads = malloc(num_pairs * sizeof(pthread_t));
    pthread_t *consumer_threads = malloc(num_pairs * sizeof(pthread_t));

    /* Setup - each producer/consumer pair shares a ring */
    for (int i = 0; i < num_pairs; i++) {
        producer_args[i].ring = &channel->rings[i];
        producer_args[i].cpu_id = i;
        consumer_args[i].ring = &channel->rings[i];
        consumer_args[i].cpu_id = num_pairs + i;  /* Consumers on different CPUs */
    }

    uint64_t start = get_time_ns();

    /* Start consumers first */
    for (int i = 0; i < num_pairs; i++) {
        pthread_create(&consumer_threads[i], NULL, consumer_thread, &consumer_args[i]);
    }

    /* Start producers */
    for (int i = 0; i < num_pairs; i++) {
        pthread_create(&producer_threads[i], NULL, producer_thread, &producer_args[i]);
    }

    /* Wait for producers to finish */
    for (int i = 0; i < num_pairs; i++) {
        pthread_join(producer_threads[i], NULL);
    }

    /* Close rings to signal consumers */
    for (int i = 0; i < num_pairs; i++) {
        ring_close(&channel->rings[i]);
    }

    /* Wait for consumers */
    for (int i = 0; i < num_pairs; i++) {
        pthread_join(consumer_threads[i], NULL);
    }

    uint64_t end = get_time_ns();
    double elapsed_ns = (double)(end - start);

    size_t total_received = 0;
    for (int i = 0; i < num_pairs; i++) {
        total_received += consumer_args[i].messages_received;
    }

    double throughput = (double)total_received / elapsed_ns;  /* msgs per ns = B/s */

    printf("│ %dP%dC        │ %8.2f B/s  │\n", num_pairs, num_pairs, throughput);

    free(channel);
    free(producer_args);
    free(consumer_args);
    free(producer_threads);
    free(consumer_threads);
}

int main(void) {
    printf("\n");
    printf("═══════════════════════════════════════════════════════════════════════════════\n");
    printf("║                       RINGMPSC C - THROUGHPUT BENCHMARK                     ║\n");
    printf("═══════════════════════════════════════════════════════════════════════════════\n");
    printf("Config: %lluM msgs/producer, batch=%dK, ring=%lluK slots\n\n",
           (unsigned long long)(MESSAGES_PER_PRODUCER / 1000000),
           BATCH_SIZE / 1024,
           (unsigned long long)(RING_CAPACITY / 1024));
    printf("┌─────────────┬───────────────┐\n");
    printf("│ Config      │ Throughput    │\n");
    printf("├─────────────┼───────────────┤\n");

    /* Warmup */
    run_benchmark(4);

    int configs[] = {1, 2, 4, 6, 8};
    int num_configs = sizeof(configs) / sizeof(configs[0]);

    for (int i = 0; i < num_configs; i++) {
        run_benchmark(configs[i]);
    }

    printf("└─────────────┴───────────────┘\n");
    printf("\nB/s = billion messages per second\n");
    printf("═══════════════════════════════════════════════════════════════════════════════\n\n");

    return 0;
}
