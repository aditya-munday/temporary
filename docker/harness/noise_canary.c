/* ccx host-noise canary.
 *
 * A fixed, self-contained workload measured immediately before and after a
 * solution's measurement. The source and the work it does never change, so any
 * drift in its wall clock between the two runs is host noise — a busy
 * neighbour, a frequency-policy change, a machine that throttled mid-run — and
 * not a property of the solution.
 *
 * The workload is intentionally modest: it allocates and touches a few
 * megabytes and does a bounded amount of integer and floating-point work, so it
 * finishes in milliseconds while still reacting to CPU contention. It writes a
 * single line to stdout so the runner sees a clean, non-empty result, and it
 * takes no input so the two runs are byte-identical in behaviour.
 *
 * This file is compiled with the same toolchain profile as the solution, but it
 * is never part of a solution's metrics: only its wall clock is read, and only
 * to label the capsule's indicative numbers as trustworthy or not.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CANARY_BYTES (32u * 1024u * 1024u)
#define CANARY_ROUNDS 2000000u

static unsigned long long mix(unsigned long long x) {
    x ^= x >> 33;
    x *= 0xff51afd7ed558ccdULL;
    x ^= x >> 33;
    x *= 0xc4ceb9fe1a85ec53ULL;
    x ^= x >> 33;
    return x;
}

int main(void) {
    unsigned char *buffer = malloc(CANARY_BYTES);
    if (buffer == NULL) {
        fputs("canary: allocation failed\n", stderr);
        return 1;
    }

    /* Touch every page, then re-read it: a memory-latency component that
       responds to cache and memory contention on the host. */
    memset(buffer, 0x5a, CANARY_BYTES);
    unsigned long long checksum = 0;
    for (unsigned i = 0; i < CANARY_BYTES; i++) {
        checksum += buffer[i];
    }

    /* A bounded compute component: enough to react to CPU contention, small
       enough to stay well under a millisecond-scale budget on a quiet host. */
    double accumulator = 0.0;
    for (unsigned round = 0; round < CANARY_ROUNDS; round++) {
        unsigned long long value = mix((unsigned long long)round + checksum);
        accumulator += (double)(value & 0xffffu) * 1e-6;
    }

    free(buffer);
    printf("canary %llu %.6f\n", checksum, accumulator);
    return 0;
}
