# CAPSULE_SPEC.md — Experiment Capsule, version 17

A **capsule** is the unit of evidence on CodeCurrency. It is a small, versioned
JSON document that fully describes one measured run of one solution to one
problem, such that anyone can re-run it and get the same answer.

Design rules:

1. **Self-describing.** Everything needed to reproduce is inside the capsule.
   No external lookup, no "you also need my local config".
2. **Content-addressed.** Source files and the problem statement are stored as
   SHA-256 hashes; the capsule references the content, it does not embed it.
3. **Pinned, never floating.** Toolchains are referenced by image digest. A
   version string is documentation; a digest is the contract.
4. **Labels are mandatory.** Every metric carries `kind`: `measured` (a real
   hardware or OS counter), `simulated` (a deterministic model, e.g. cachegrind),
   or `indicative` (wall-clock, machine-dependent).
5. **Validated on load.** A capsule that does not validate is rejected, not
   best-effort parsed. Unknown fields are a hard error at v1.
6. **Versioned migrations.** The schema only changes by adding a new version and
   a migration from the previous one. Old capsules never become unreadable.

---

## 1. Top level

```jsonc
{
  "schema_version": 17,         // integer, bumped only with a migration
  "capsule_id": "…",            // sha256 of the canonical body (see §10)
  "created_at": "2026-10-26T11:00:00Z",  // RFC 3339, UTC
  "problem":   { … },           // §2
  "source":    { … },           // §3
  "toolchain": { … },           // §4
  "build":     { … },           // §5
  "reference_build": { … },     // §6, optional for non-paired legacy capsules
  "harness":   { … },           // §7
  "input":     { … },           // §8
  "host":      { … },           // §9
  "verdict":   "accepted",      // §11
  "metrics":   [ … ],           // §12
  "repeatability": { … },       // §13, optional
  "build_repeatability": { … }, // §14, optional
  "applied_tolerance": { … },   // §16, optional
  "noise_canary": { … }         // §17, optional
}
```

## 2. `problem`

| field | type | notes |
|---|---|---|
| `id` | string | slug, e.g. `top-k/heap-vs-sort` |
| `statement_sha256` | string | hex, of the canonical statement file |

## 3. `source`

| field | type | notes |
|---|---|---|
| `language` | enum | `c`, `cpp`, `asm-x86-64`, `rust`, `d`, `zig`, `odin`, `go` (current selectable build set; `asm-x86-64` is written as `asm` in `ccx.toml`) |
| `entry` | string | relative path of the translation unit to compile |
| `files` | map<string,string> | relative path → `sha256:<hex>`; `problem/` paths are relative to the problem folder, others to the solution folder |
| `source_sha256` | string | hex, over the sorted `files` map (§10) |

`asm-x86-64` remains a legacy schema value for decoding older capsules. New
solution configurations and profile catalogs do not expose assembly as a
selectable language; `ccx asm` reports compiler-generated assembly instead.

## 4. `toolchain`

| field | type | notes |
|---|---|---|
| `name` | enum | `gcc`, `gcc-13`, `clang`, `clang-18`, `rust`, `gdc`, `zig`, `odin`, `go` |
| `version` | string | compiler-reported version, documentation only |
| `image` | string | locally built image name, e.g. `ccx-gcc` |
| `image_digest` | string | `sha256:…`, the contract |
| `environment_fingerprint` | string\|null | stable fingerprint of the pinned build recipe, packages, tool binaries, and harness; null only for migrated legacy capsules |
| `libc` | string | e.g. `glibc 2.41` |

`image_digest` records the exact local OCI image used to create the capsule.
Locally built image IDs can differ across hosts because the OCI config records
build timestamps. New capsules also record `environment_fingerprint`. Replay
uses that fingerprint to accept a separately built local image only when its
pinned base, package snapshot, Dockerfile, tool binaries, and harness match.
Legacy capsules without a fingerprint still require their exact image digest.

## 5. `build`

| field | type | notes |
|---|---|---|
| `argv` | string[] | full compiler argv after the allow-list check |
| `opt_level` | string | e.g. `-O2` (extracted, for grouping) |
| `binary_sha256` | string | `sha256:<hex>` of the measured executable, or `unavailable_legacy` on migrated capsules that predate executable digests |
| `build_repeat` | integer | number of clean candidate builds; every build must produce identical executable bytes |
| `env` | map<string,string> | *fixed* environment (see §15) |
| `argv_program` | string[] | program arguments passed at run time |
| `argv_runner` | string[] or omitted | optional runtime command argv with `{binary}` and `{args}` expanded |
| `stdin_sha256` | string\|null | hash of piped stdin, null if none |
| `stack_bytes` | integer | `RLIMIT_STACK`, fixed across runs |

## 6. `reference_build`

Present on new capsules that record paired reference timing.

| field | type | notes |
|---|---|---|
| `binary_sha256` | string | `sha256:<hex>` of the exact reference executable used for paired timing |
| `argv` | string[] | full reference compiler argv inside the pinned image |

The problem reference source is included in `source.files` under
`problem/reference.cpp`. Replay checks both the rebuilt reference digest and
compiler argv before comparing paired timing. Schema-12 paired capsules migrate
without a fabricated reference digest; replay calls out that missing identity.

## 7. `harness`

| field | type | notes |
|---|---|---|
| `version` | string | semver of the measurement harness |
| `metric_method` | enum | `cachegrind`, `perf`, `getrusage`, `stat`, `manual`, `valgrind_memcheck`, `wall_clock`, `runner` |
| `ruleset` | string | hash of the rule set used for hints (§Layer 8) |

## 8. `input`

| field | type | notes |
|---|---|---|
| `generator` | string | id of the seeded generator |
| `seed` | integer | the seed actually used |
| `seed_set` | enum | `public` (practice) or `hidden` (ranking) |
| `size` | integer\|null | problem-specific scale parameter |

## 9. `host`

| field | type | notes |
|---|---|---|
| `cpu_model` | string | from `/proc/cpuinfo` |
| `arch` | string | `x86_64` |
| `os` | string | `/etc/os-release` `PRETTY_NAME` |
| `kernel` | string | `uname -r` |
| `nproc` | integer | usable CPUs at run time |
| `frequency_policy` | string | governor, or `unknown` |
| `fingerprint` | string | hex, hash of the above (§10) |

`host` is recorded but **must not affect** deterministic metrics. If it does,
that is a bug in the metric, not a property of the host.

## 10. Hashing

All hashes are SHA-256 and lowercase hex. `source.files` and recorded
`build.binary_sha256` values use the `sha256:<hex>` form. The literal
`unavailable_legacy` is a migration marker, not a hash; new measurements always
write an executable digest.

- `source_sha256`: over the UTF-8 bytes of `"<path>\0<hex>\n"` for each entry,
  paths sorted lexicographically ascending.
- `build.binary_sha256`: over the exact executable bytes copied from the build
  sandbox and then used for measurement.
- `host.fingerprint`: over `"<cpu_model>|<arch>|<os>|<kernel>|<nproc>"`.
- `capsule_id`: over the canonical JSON of the capsule body with `capsule_id`
  removed and all object keys sorted lexicographically, no insignificant
  whitespace.

Replay rebuilds the candidate and reference executables and rejects a digest
mismatch before measuring. The cross-machine repeatability check also requires
the same candidate and reference executable digests and reference compiler argv
on every host. Capsules marked `unavailable_legacy` can still replay, but
cannot prove that the original candidate executable bytes were identical.

`capsule_id` therefore changes if *any* input to the experiment changes, and
never changes if nothing does.

## 11. Verdict model

One of: `accepted`, `wrong_answer`, `time_limit`, `memory_limit`,
`runtime_error`, `build_error`, `rejected_unsafe`.

A capsule with any verdict other than `accepted` may omit `metrics` for the
run itself but must still record the attempt.

## 12. `metrics`

An array so that a single run can carry several metrics and so the shape stays
stable as metrics are added.

```jsonc
{
  "name": "instructions",
  "value": 1234567,
  "unit": "count",
  "kind": "simulated",        // measured | simulated | indicative
  "method": "cachegrind",     // cachegrind | perf | getrusage | stat | manual | valgrind_memcheck | wall_clock | runner
  "uncertainty": null          // or { "kind": "stdev"|"ci95"|"range", "value": … }
}
```

Phase 1 metric set:

| name | unit | kind | method |
|---|---|---|---|
| `instructions` | count | simulated | cachegrind |
| `cache_misses` | count | simulated | cachegrind |
| `instruction_cache_misses`, `data_cache_misses` | count | simulated | cachegrind |
| `data_refs`, `data_reads`, `data_writes` | count | simulated | cachegrind |
| `branch_instructions` | count | simulated | cachegrind |
| `cache_miss_rate`, `branch_miss_rate` | ratio | simulated | cachegrind |
| `instructions_hw` | count | measured when available | perf |
| `cache_misses_hw` | count | measured when available | perf |
| `branch_misses_hw` | count | measured when available | perf |
| `cpu_cycles_hw` | count | measured when available | perf |
| `l1d_cache_misses_hw`, `l1i_cache_misses_hw`, `llc_cache_misses_hw` | count | measured when available | perf |
| `dtlb_cache_misses_hw`, `itlb_cache_misses_hw` | count | measured when available | perf |
| `ipc_hw` | instructions/cycle | measured when available | perf |
| `average_cpu_frequency_hw` | hertz | measured when available | perf + getrusage |
| `branch_misses` | count | simulated | cachegrind |
| `peak_memory` | bytes | indicative | getrusage |
| `allocations` | count | measured | Valgrind Memcheck xtree |
| `allocated_bytes` | bytes | measured | Valgrind Memcheck xtree |
| `binary_size` | bytes | measured | stat |
| `wall_clock` | nanoseconds | indicative | runner monotonic clock |
| `reference_wall_clock` | nanoseconds | indicative | runner monotonic clock |
| `paired_speedup_ratio` | ratio | indicative | paired runner wall-clock samples |
| `stdout_bytes`, `stderr_bytes` | bytes | measured | runner byte counters |
| `user_cpu_time`, `system_cpu_time` | nanoseconds | measured | `getrusage` |
| `minor_page_faults`, `major_page_faults` | count | measured | `getrusage` |
| `voluntary_context_switches`, `involuntary_context_switches` | count | measured | `getrusage` |
| `block_input_ops`, `block_output_ops` | count | measured | `getrusage` |
| `build_wall_clock` | nanoseconds | measured | runner monotonic clock |
| `build_user_cpu_time`, `build_system_cpu_time` | nanoseconds | measured | `getrusage` |

Deterministic metrics are the ranking signal. `wall_clock` is explicitly
`indicative` and is **never** used for ranking in Phase 1.

`measured` identifies a real instrument; it does not by itself promise
cross-machine determinism. `getrusage` process counters are measured OS values
and remain machine-dependent. Replay compares them using their recorded
intervals, while cross-machine equality checks exclude them.

The execution resource metrics describe the direct child process reported by
Linux `wait4(2)`. Build resource metrics describe the compiler or build-driver
process tree. Block-I/O values count operations and are not byte counts.
`build_wall_clock` measures the clean build command inside the runner; it
excludes Docker startup and artifact transfer. Network latency, file-level I/O
latency, and live CPU-frequency time series are not part of this harness
version. An effective average frequency may be derived from hardware cycles
when available.

Harness version 0.13.0 fixes Cachegrind's I1 and D1 caches at 32 KiB, 8-way,
64-byte lines. Cachegrind 3.24 uses the host-detected last-level cache even when
an LL geometry is provided; LL misses are excluded from reported metrics.
Earlier harness versions allowed Cachegrind to detect I1 and D1 cache sizes
from the host CPU and must not be used for cross-machine cache-miss comparisons.
Harness version 0.18.0 accepts a complete Memcheck xtree profile whose totals
are explicitly `(null)` as zero allocations and bytes. A missing, malformed, or
duplicate totals section remains unavailable and fails measurement.

Harness version 0.19.0 writes and validates the raw execution and clean-build
repeatability observations in the schema-15 `values` arrays. The cross-machine
gate compares these vectors exactly for deterministic metrics. Schema-14
capsules migrate without fabricated observations and retain their original
harness version.

Harness version 0.17.0 validates each traced process summary and sums all
process counters instead of retaining only the final summary.

`ccx measure --repeat N` builds the candidate three times, requires identical
executable bytes, and measures that exact executable for all N samples. Each
wall-clock sample runs the candidate and the problem's C++
`reference.cpp` consecutively; their execution order alternates by sample. The
capsule records each duration and the per-pair ratio `reference_wall_clock /
wall_clock`, where a value above 1 means the candidate was faster than the
reference on that pair. These indicative ratios have a seeded bootstrap
interval. Because the reference is C++, ratios for other languages are a
cross-language context only, not a same-language ranking signal. Build time is
the median of three measured, host-dependent clean builds, with observed spread
and a seeded bootstrap interval. Replay repeats the recorded candidate build
count, checks that rebuilt candidate bytes match, and repeats the same
alternating paired measurements against those bytes. It also checks the
reference compiler argv and reference executable digest recorded in
`reference_build`.

A metric is omitted when its instrument is not available for the selected
language/runtime. Missing metrics must never be filled with zero. The current
GDC, Zig, Odin, and Go profiles omit allocator metrics because collection has
only been enabled and validated for C, C++, and Rust. Go assembly is produced
from the built binary with `go tool objdump`; source mapping follows the Go
tool's emitted source locations.

Allocation metrics use a separate Memcheck process and an xtree report framed
by the runner. The runner rejects missing or malformed reports, reads only the
main Valgrind process report, and terminates detached descendants before
reading it. Integration probes verify that direct report forgery and delayed
forked writes are rejected. The current collector counts threads in the main
process but does not aggregate allocations from forked or exec'd child
processes, custom allocators Memcheck cannot recognize, or peak live bytes.

## 13. `repeatability`

Present when a metric was measured more than once.

```jsonc
{
  "runs": 20,
  "per_metric": {
    "instructions": {
      "samples": 20,
      "values": [ … ],
      "min": …,
      "max": …,
      "median": …,
      "spread_pct": 0.0,
      "p95": …,
      "p99": …
    }
  }
}
```

`spread_pct = (max - min) / max(abs(median), 1) * 100`. The one-unit floor
keeps near-zero OS counters finite and makes their absolute variation visible.
`p95` and `p99` are nearest-rank quantiles of the observed samples. They are
descriptive values, not confidence bounds; old capsules migrated from earlier
schemas leave them absent rather than inventing tail data. A small run count
may put p99 at the observed maximum.
`values` stores every raw observation in execution order. Its length must equal
`samples`, all values must be finite, and the loader recomputes and validates
the summary fields from the raw observations. Schema-14 capsules migrate with
`values` absent because the original observations cannot be reconstructed.
Replay repeats the same number of runs and compares medians using twice the
combined measured 95% confidence half-width. A match inside that interval is
statistical, not bit-for-bit equality; zero-uncertainty metrics still require
exact equality.
Run counts range from 2 to 100; Go capsules require at least 20 runs because
its current fresh-process counter spread is wider. The latest completed
hosted three-runner gate passed for C, C++, Rust, D, Zig, and Odin under
harness 0.19.0. It validated each capsule's raw samples and summaries and
confirmed exact deterministic sample-vector agreement across the three hosted
Ubuntu runners. Go remains diagnostic because its samples exceed the
within-host tolerance. These hosted runners do not establish repeatability
across three bare-metal CPU models.

## 14. `build_repeatability`

Present when the capsule records more than one clean candidate build. The
current measurement command performs three builds; every executable SHA-256
digest must match before any result is accepted. `build.build_repeat` records
the count. The summary has the same `runs` and per-metric `samples` fields as
execution repeatability, including ordered raw `values`, and records the
median, observed range, nearest-rank p95 and p99, and a seeded bootstrap 95%
confidence interval for each build metric. Build timings are measured and
host-dependent; they are not ranking
signals or cross-host determinism gates. Older capsules migrate with
`build_repeat: 1` and no fabricated build repeatability data.

## 15. Fixed environment

Compilation and execution both run with a **fixed** environment so counts do not
drift between runs. The canonical set (all other variables are dropped):

```
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
LANG=C
LC_ALL=C
TZ=UTC
SOURCE_DATE_EPOCH=0
```

## 16. `applied_tolerance`

Optional. Present only when the measured profile declares a bounded tolerance
for a simulated metric whose value carries a small, reproducible per-host
offset. Two policies are defined:

- `d-gdc-druntime-cpuid`: the D druntime calls `core.cpuid` at startup, so
  Cachegrind counts for `gdc` depend on the host CPU model even inside the
  pinned image.
- `rust-libc-ifunc-dispatch`: the Rust runtime and libc resolve CPU-feature
  (ifunc) variants before `main`, so a differing host CPU model adds a small
  fixed offset to the simulated counts for `rust`.

Both cover the thirteen simulated Cachegrind metrics at a `1e-3` (0.1%)
relative bound. The bound is ten times the 0.01% within-host gate, because it
admits a fixed per-host offset rather than within-host noise, yet it sits just
above the largest offset the hosted runners have shown (0.043% on the Rust/C++
`branch_misses` count).

```jsonc
{
  "policy": "rust-libc-ifunc-dispatch", // id of the named policy
  "toolchain": "rust",                  // toolchain the policy applies to
  "relative_tolerance": 1e-3,           // policy bound on cross-host relative spread
  "metrics": [ "instructions", … ],     // the simulated metrics the policy covers
  "observed_relative_spread": 0.0       // this capsule's own observed spread
}
```

Rules:

- `policy` must name a known policy, and `toolchain` must match that policy.
- `relative_tolerance` may not exceed the policy's own bound.
- Every entry in `metrics` must be a simulated metric that the capsule records.
- The cross-machine comparison relaxes the exact-equality gate only for the
  metrics a policy names, only up to the policy bound, and only when all three
  hosts declare the same policy. Raw samples must still be constant within each
  host, so the tolerance admits a per-host offset, not within-host noise.
- `ccx replay` applies the same recorded bound to a simulated metric, so a
  capsule replayed on a different CPU model is not reported as a lie.
- Measured metrics and profiles without a policy still require exact agreement.

## 17. `noise_canary`

Optional. Records the result of a fixed host-noise canary measured immediately
before and after the solution's own runs. The canary is a small, self-contained
C workload (`docker/harness/noise_canary.c`, identified by
`workload: "ccx-canary-v1"`) that allocates and touches a fixed buffer and does
a bounded amount of integer and floating-point work. Its source and its work
never change, so any drift between its two runs is a property of the host — a
busy neighbour, a frequency-policy change, a machine that throttled mid-run —
and not of the solution.

The canary is **advisory**. It never changes a metric, never changes a verdict,
and never fails a measurement: a canary that cannot run is recorded as
`unavailable` with a reason. Its only purpose is to tell a reader whether the
capsule's indicative numbers (wall clock, CPU time, RSS) were taken on a quiet
host.

```jsonc
{
  "workload": "ccx-canary-v1",   // stable id of the fixed workload
  "before_wall_ns": 27000000.0,  // canary wall clock before the solution's runs
  "after_wall_ns": 27500000.0,   // canary wall clock after the solution's runs
  "drift_pct": 1.85,             // relative drift between the two, in percent
  "tolerance_pct": 20.0,         // drift bound the status was judged against
  "status": "stable",            // "stable" | "drifted" | "unavailable"
  "note": "…"                    // present only when status is "unavailable"
}
```

Rules:

- `workload` is a non-empty identifier of the fixed workload.
- `before_wall_ns` and `after_wall_ns` are non-negative, finite nanosecond
  samples; `drift_pct` and `tolerance_pct` are non-negative and finite.
- `drift_pct` must equal the relative drift of the two samples, and `status`
  must follow from it: `stable` when the drift is within `tolerance_pct`,
  `drifted` otherwise. A capsule whose status disagrees with its own numbers is
  rejected, so the label can never be hand-waved onto a bad measurement.
- `status: "unavailable"` must carry a `note` explaining why the canary could
  not run, and records zeroed samples. The default tolerance is 20%, deliberately
  generous: the canary catches gross drift, not micro-noise, so it does not
  become a second, hidden flake source.
- `ccx replay` surfaces a `drifted` or `unavailable` canary from the original
  measurement as a note, so a reader is warned before trusting indicative
  numbers.

## 18. Migration policy

- Adding an optional field with a safe default: keep the version, note it in the
  changelog. Readers must tolerate unknown *optional* additions only where the
  field is declared `optional`; otherwise unknown fields are rejected.
- Any other change: increment `schema_version`, ship `migrate_v{n}_to_v{n+1}`,
  and keep old capsules loadable by migrating on read.
- A capsule is never silently rewritten on disk by a migration; migration output
  is written to a new file.

Every step below ships as `migrate_v{n}_to_v{n+1}` in
`crates/ccx-core/src/capsule/migrate.rs`, is covered by a test, and only ever
adds a safe default. "Set version" means the step only bumps the integer.

| From | To | What the step adds | How old capsules migrate |
|------|----|--------------------|--------------------------|
| 1 | 2 | `wall_clock` metric method | set version |
| 2 | 3 | Rust language variant | set version |
| 3 | 4 | D and GDC variants | set version |
| 4 | 5 | Zig and its pinned toolchain | set version |
| 5 | 6 | Odin and its pinned toolchain | set version |
| 6 | 7 | Go and its pinned toolchain | set version |
| 7 | 8 | distinct GCC and Clang compiler profiles | set version |
| 8 | 9 | runner-collected stdout/stderr byte counts; optional expanded runtime argv | set version; capsules without an expanded argv use `argv_program` with the fixed binary path |
| 9 | 10 | stable toolchain environment fingerprint | older capsules are marked as lacking the fingerprint (null) |
| 10 | 11 | observed p95 and p99 repeatability quantiles | older repeatability data keeps those fields absent |
| 11 | 12 | measured executable SHA-256 | set to `LEGACY_BINARY_DIGEST_UNAVAILABLE` |
| 12 | 13 | paired reference executable SHA-256 and compiler argv | no fabricated reference provenance |
| 13 | 14 | candidate build count and clean-build timing spread | one honest build; legacy build-time intervals are removed and kept as single values |
| 14 | 15 | raw execution and build repeatability observations | summaries are retained without fabricated raw samples |
| 15 | 16 | optional `applied_tolerance` | the field is simply absent |
| 16 | 17 | optional `noise_canary` | the field is simply absent |

Original fields keep their meaning across every step. A capsule is never
silently rewritten on disk by a migration; migration output is written to a new
file.
