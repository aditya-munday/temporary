# Changelog

## [Unreleased]

- Make the measurement trust contract mechanical, not aspirational (Part 1).
  The five metric kinds — `measured`, `simulated`, `indicative`, `predicted`,
  and the absence outcome `unavailable` — now live as data in
  `ccx-core::trust` (`KIND_SPECS`, `PRINCIPLES`, `check_invariants`), the same
  data the tests assert, so the taxonomy cannot drift from the code that
  enforces it. `ccx principles` prints it and `ccx principles --check` exits
  non-zero when an invariant is violated; CI runs the check as the Part 1 gate.
  `predicted` (schema 20) is the kind for a static model of code that never ran,
  and `llvm-mca` its method, so `ccx throughput` can name what it does without
  borrowing the word "measured". The ranking rule is decided in one place
  (`Metric::rankable_kind` + `ranking_eligibility`): only `measured` and
  `simulated` may rank, and a `predicted` or `indicative` entry is refused with
  a reason. Schema 20 adds only the two new enum values; an old capsule's
  `kind`/`method` labels still parse and are not rewritten.

- Stop the allocation hint from printing `totalling 0 bytes` when the byte
  total was not recorded. `allocations` and `allocated_bytes` are separate
  metrics; when the byte total is absent the hint now says so instead of
  inventing a zero, which the trust contract forbids. A regression test covers
  it.

- Add `ccx topdown`, the top-down microarchitecture breakdown (C11). It runs the
  solution under `perf stat --topdown` and reports the four level-one buckets —
  retiring, bad speculation, front-end bound, back-end bound — whose fractions
  sum to one, plus the raw perf metric strings so the parse can be checked. The
  numbers come from the hardware PMU, so they are labelled `measured` but are
  machine-dependent and are never a ranking signal, exactly like the flat
  hardware counters the capsule already carries. A PMU that refuses the events
  or does not expose the slot counters yields an honest `unavailable` with
  perf's own diagnostic, never zeros. The analyzer lives in the pinned Clang
  image, which now carries `linux-perf`, so the solution's own toolchain does
  not need it.

- Add `ccx throughput`, the static throughput a machine model predicts for the
  solution's assembly (C12). It compiles the measured build's assembly and reads
  it with `llvm-mca`, reporting the modelled cycles, IPC, uOps per cycle,
  dispatch width, and the resource pressure aggregated busiest-first, plus the
  instructions the model does not understand. Because nothing ran, the result is
  labelled `predicted` and is never a measurement or a ranking signal. The
  report always names the CPU model it modelled against. `llvm-mca` is an LLVM
  tool, so the analyzer lives in the pinned Clang image; the assembly is emitted
  in AT&T syntax because the analyzer rejects GCC's Intel-syntax mnemonics.
  Sub-resource names that llvm-mca emits as raw control bytes are rendered as
  their decimal index (`Zn4LSU.2`) with a note.

- Read peak memory from the cgroup high-water mark when the kernel exposes one
  that covers the run, falling back to `getrusage` peak RSS otherwise (schema
  19). The runner reads `memory.peak` (cgroup v2) or `memory.max_usage_in_bytes`
  (v1) around the child, rejects a reading that a host-scoped cgroup already
  reached before the run, and records which source produced the number as
  `peak_memory_source`. The `peak_memory` metric keeps its name and gains a
  `cgroup` method alongside `getrusage`, so a capsule never labels a kernel
  accounting number as a process RSS number or the reverse. Both remain
  `indicative` and outside the deterministic match. Schema 19 adds only the new
  method value; an old capsule keeps the `getrusage` method that actually
  produced its number.

- Add `ccx latency`, which surfaces the orphaned latency-statistics core. It
  reads the raw wall-clock samples a capsule records and reports the distribution
  its median and spread hide: min, median, mean, max, standard deviation,
  percentiles, and a log2 histogram. It never interpolates: a percentile the
  sample count cannot support is reported as `unavailable` with the count it
  would need, so a twenty-run capsule cannot invent a p99. The whole view is
  labelled `indicative`, because run latency is a host-dependent wall-clock
  measurement.

- Add `ccx scaling`, which surfaces the orphaned thread-scaling core. It runs one
  solution at a sweep of thread counts (powers of two up to the host's logical
  CPU count by default), re-checks correctness at every count, and reports the
  median wall time, the speed-up against the one-thread baseline, and the
  efficiency. It never extrapolates a count it did not run and reports where the
  curve saturates. Speed-up is a wall-clock ratio and is therefore `indicative`;
  a serial solution correctly yields a flat curve. The sweep overrides the
  toolchain's own thread pin (Go's `GOMAXPROCS=1`) rather than shadowing it.

- Record wall-clock timing diagnostics (schema 18). `ccx measure` now classifies
  its indicative wall-clock samples in an optional `timing` capsule field: each
  sample carries its run index and, when it sits above twice the median or below
  half of it, a slow/fast outlier flag, alongside a trimmed median and warm-up
  evidence. The warm-up evidence is derived from the samples already recorded —
  how many leading runs were slow and how far the first run sat above the median
  — so it adds no runs and cannot change a measurement. `ccx replay` surfaces a
  host that had not settled, or flagged outliers, as an advisory note. The
  diagnostic never affects a metric, a rank, or a verdict.

- Pin Zig's build job count to one. Zig's emitted code depends on its job
  count, which otherwise follows the host's detected CPU count, so the same
  source built to different bytes on a 2-vCPU CI runner and a 4-vCPU
  workstation. The clean-build reproducibility gate now holds across hosts.

## [0.1.0] - 2026-11-02

- Harden the capsule parser and prove it. New `capsule_robustness` tests cover
  migration idempotency, rejection of unknown and future versions, a path from
  every historical version to the current one, goldens for every language, and a
  fuzzer that feeds garbage, truncated, and structurally wrong documents to the
  parser. The fuzzer found a real panic: `migrate_value` indexed a nested field
  without checking it was an object. `migrate.rs` now uses total `get`/`set`
  accessors, so a malformed document is a typed error, never a panic.

- Widen the hostile suite from eight cases to seventeen. New cases cover a
  spawning tree, descriptor exhaustion, a scratch symlink pointing at a host
  path, `/proc` escape hatches and raw kernel memory, `ptrace`, clock tampering,
  stderr flooding, signal tricks (a child sending `SIGKILL`/`SIGSTOP`/`SIGSEGV`
  to the runner, which is PID 1 and survives because the sandbox drops
  `CAP_KILL`), and output forgery (a child printing the runner's own
  `CCX-VERDICT` framing, which the length-framed report traps inside the
  `CCX-OUT` section so the real status still wins). `scripts/safety-suite.sh`
  runs all seventeen against the real pinned image and asserts containment by
  reason.

- Name the planned language slots honestly. The capability matrix now lists Nim,
  Ada, Fortran, and ARM64 assembly as `planned` rows with every capability `no`,
  `ccx init` rejects them with the reason they need a pinned image, runner
  support, and a problem fixture, and `docs/LANGUAGES.md` documents the status
  column and how a slot is promoted to `available`.

- Make a non-x86-64 host fail clearly. `ccx doctor` now reports a `host platform`
  check that is `MISS` on any architecture other than x86-64, with the reason
  (the pinned execution images are x86-64) and the fix, instead of letting
  Docker fall back to slow, non-deterministic emulation.

- Add `.devcontainer/devcontainer.json` and `scripts/devcontainer-setup.sh` so a
  Codespace or devcontainer comes up with Docker-in-Docker, Rust 1.85.1, and the
  pinned toolchain images already built and `ccx doctor` ready.

- Move the standalone guides under `docs/` (`AUTHORING.md`, `LIMITATIONS.md`,
  `METHODOLOGY.md`, `PERFORMANCE_FINDINGS.md`, `PLATFORM_SUPPORT.md`) and update
  every reference in the README, CONTRIBUTING, and the source comments.

- Add a release pipeline (`.github/workflows/release.yml`). Pushing a `v*` tag
  builds `ccx` for x86-64 Linux, x86-64 macOS, Apple Silicon macOS, and x86-64
  Windows, packages each as a `tar.gz`/`zip` with the README, LICENSE,
  CHANGELOG, and capsule spec, writes a `SHA256SUMS` file, and creates a GitHub
  release whose notes are the matching `CHANGELOG.md` section.

- Add a supply-chain gate and an MSRV gate to CI. `cargo deny check advisories
  licenses bans sources` runs against a checked-in `deny.toml`, and a separate
  job builds and tests the workspace on the declared MSRV (1.85.0). The
  `ccx-core` path dependency now carries a version so the wildcard-dependency
  ban holds.


- Add `ccx repeat`, the reproducibility report. It runs `measure` N separate
  times, each producing its own capsule, and reports the spread across those
  capsules for every deterministic metric against the ranking tolerance
  (`0.01%`). It prints the tolerance claimed and the tolerance achieved, lists
  each metric's min/max/median/spread with an `exact` marker, counts the
  machine-dependent metrics it deliberately does not gate, and exits non-zero
  when any deterministic metric drifts beyond the tolerance. `measure --repeat`
  shows the spread inside one session; `repeat` shows whether the solution
  reproduces at all. Wired into `container-engine` CI.

- Document the sandbox threat model in `SECURITY.md`: the trusted/untrusted
  split, every `docker run` control and its purpose, result-integrity rules, the
  eight-case hostile suite, and the explicit non-goals (Docker/kernel escapes,
  compromised toolchain images, cross-run denial of service, side channels).
  Link it from the README trust section. Update `AGENTS.md` to the merged state
  (main at `79ae23f`, all three Phase 1 PRs landed) and refresh the verified
  test counts to 211 (148 core + 63 CLI).

- Add a host-noise canary. `ccx measure` now runs a fixed, self-contained C
  workload (`ccx-canary-v1`) immediately before and after the solution's own
  runs, records both wall clocks, the relative drift, the tolerance, and a
  `stable` / `drifted` / `unavailable` status in the capsule's new
  `noise_canary` field. The canary is advisory: it never changes a metric or a
  verdict, and a canary that cannot run is recorded as `unavailable` with a
  reason instead of failing the measurement. `ccx replay` surfaces a `drifted`
  or `unavailable` canary from the original measurement as a note, and `ccx
  measure` prints the status. This advances capsules to schema 17; v16 capsules
  migrate by leaving the field absent.

- Record the portable cache-hierarchy and TLB read-miss hardware counters:
  `l1d_cache_misses_hw`, `l1i_cache_misses_hw`, `llc_cache_misses_hw`,
  `dtlb_cache_misses_hw`, and `itlb_cache_misses_hw`. The runner opens them
  through `PERF_TYPE_HW_CACHE` alongside the existing hardware events, the
  engine parses them as optional `measured` metrics, and `ccx doctor` reports
  them. They are absent, not zero, when the host or sandbox cannot open the
  event, and they never replace the simulated Cachegrind counters. The generic
  `perf` cache interface exposes no portable L2 event, so the LLC counter stands
  in for the outermost cache level.
- Regenerate the three published pair-counting evidence capsules (naive, sort
  and two pointers, and count and prefix sum) for the refreshed runner. Opening
  the new hardware events changes the runner binary, which changes the pinned
  image digest and the toolchain environment fingerprint, so capsules recorded
  under the previous runner no longer satisfy replay's environment check. The
  deterministic Cachegrind, Memcheck, and binary-size metrics are byte-for-byte
  unchanged; only the indicative wall-clock and build-time samples moved, and
  all three capsules replay with `deterministic_match: true`.
- Rank compiler profiles into statistical tie groups in `ccx matrix`. Two
  profiles whose bootstrap intervals overlap share a rank; ties are transitive,
  so a chain of overlapping intervals shares one rank rather than being split
  into pairwise claims the intervals do not support. Profiles whose
  repeatability is ineligible are excluded from the ranking, not ranked last.
  The grouping is a library function (`ccx_core::ranking::rank_groups`) with
  unit tests.
- Add `ccx pareto`, a trade-off frontier across capsules over deterministic
  metrics. It reports which capsules are dominated and which are on the
  frontier, keeps a capsule missing an axis off the frontier instead of
  treating it as infinitely bad, and refuses a wall-clock or hardware-counter
  axis because a frontier over a noisy axis is a drawing of noise. The
  dominance relation is a library function (`ccx_core::ranking::pareto_frontier`)
  with unit tests.
- Add x86-64 assembly as a selectable submission language (`language = "asm"`
  with the pinned GAS toolchain). Builds are static, use Linux syscalls
  directly, and need no libc. Build, test, measure, replay, and `ccx asm` all
  work, and `arena/asm` solves `sorted-pair-sum` with a counting sort and a
  two-pointer scan. The scaffold writes a `.file` directive so the object is
  byte-identical across clean builds. Assembly reports no allocation metric and
  no compiler remarks, and both are marked unavailable rather than zero.
- Publish the per-language capability matrix as `docs/LANGUAGES.md` and as a
  `capabilities` table in `ccx profiles` (aliases `ccx languages` and
  `ccx platforms`). The matrix covers instructions, cache and branch
  simulation, allocations, peak memory, binary size, compiler remarks, the
  assembly view, and a repeatability grade (`exact`, `bounded`, or `unranked`)
  with a reason for every gap.
- Advance capsules to schema 16. A profile may declare a bounded
  `applied_tolerance` for a simulated metric that carries a small, reproducible
  per-host offset. The cross-machine gate relaxes exact equality only for the
  named metrics, only up to the policy bound, and only when every host declares
  the same policy; measured metrics and raw within-host samples stay exact.
  Two policies are defined, each covering the thirteen simulated Cachegrind
  metrics at a `1e-3` relative bound: `d-gdc-druntime-cpuid` for `gdc`, because
  the D druntime reads `core.cpuid` at startup, and `rust-libc-ifunc-dispatch`
  for `rust`, because the Rust runtime and libc resolve CPU-feature (ifunc)
  variants before `main`. `ccx replay` applies the same recorded bound, so a
  capsule replayed on a different host CPU model still reproduces. Older
  capsules migrate without a declared tolerance and are unaffected.
- Advance capsules to schema 15 and harness 0.19.0. Preserve every execution
  and clean-build repeatability observation, validate summaries from raw data,
  and compare deterministic sample vectors across hosted machines. Migrate
  older capsules without fabricating observations.
- Report when a ranked metric's same-host repeatability exceeds the 0.01%
  tolerance or has fewer than twenty samples, and withhold compiler-matrix
  comparisons for ineligible metrics.
- Require matching problem files, input bytes, compiler environment, and
  harness before `ccx diff` labels metric changes conclusive; identify unit or
  metric-method changes as inconclusive, with a reason shown for each metric.
- Include execution architecture, operating system, runtime arguments, fixed
  environment, and stack limit in the capsule-diff comparison context.
- Include optimization level and normalized compiler/build arguments in the
  capsule-diff comparison context so flag changes cannot be attributed to code.
- Refresh ten single- and multi-file C, C++, Rust, D, Zig, Odin, and Go
  twenty-run capsules for harness 0.18.0; replay each capsule and record the
  local repeatability evidence and current Go metric spreads.
- Let independent hosted runners share a host-class fingerprint in the
  repeatability gate while still requiring three distinct runner labels.
- Advance the measurement harness to 0.18.0. Treat a structurally valid Memcheck
  xtree profile with `totals: (null)` as a measured zero-allocation result.
- Advance the measurement harness to 0.17.0, reject incomplete Cachegrind
  summaries, and sum all traced-process counters. Make the cache model claim
  precise: fixed I1/D1 misses are reported, while host-detected LL misses are
  excluded.
- Extend `ccx experiment` to include source-mapped assembly artifacts and
  compiler optimization remark status and explanations after correctness and
  measurement succeed.
- Repeat candidate clean builds three times, require identical executable
  hashes, and record build wall and CPU time distributions in schema-14 capsules;
  replay repeats the recorded build count. Legacy build-time spreads without a
  recorded build count migrate as single observations.
- Advance the profile catalog to schema version 12 for repeated build timing
  metrics and their uncertainty.
- Require every repeatability metric's sample count to match the run count and
  reject capsules requesting more than 20 candidate builds.
- Bump the runner to harness 0.15.0. Reject malformed resource limits, fail
  closed if a process limit cannot be installed, and report per-stream output
  truncation instead of silently accepting incomplete output.
- Bump the runner to harness 0.16.0. Grow output capture within the combined
  stdout and stderr cap, preserving single-stream output below the configured
  total limit.
- Regenerate and replay all ten checked-in single-file and multi-file language
  capsules against the harness 0.16.0 pinned toolchains.
- Expand all ten checked-in language capsules to twenty fresh-process samples
  and record per-language within-host instruction, cache, and wall-time spreads.
- Expand `ccx validate` to exercise all public and hidden seeds across declared
  sizes, cross-check brute force, validate each matched-size variant, and base
  size separation on median reference instruction counts.
- Run the real hostile-submission suite on the local Linux Docker host; all
  eight containment cases passed. Record the host scope and unverified
  Windows/macOS integrations in the limitations.
- Regenerate and replay the checked-in Phase 1 capsule set under harness 0.14.0.
- Measure one built executable across repeated runs; record build timing as a
  single measured sample and replay by rebuilding once, then bump the harness
  version to 0.11.0.
- Regenerate the naive, optimized, and Go sample capsules under harness 0.11.0;
  record same-binary repeatability and verify deterministic replay.
- Refresh and replay the C, C++, Rust, D, Zig, Odin, and Go single- and
  multi-file capsule examples under harness 0.11.0.
- Advance the profile catalog to schema version 10 for the measured,
  host-dependent single-observation build timing metric.
- Pair each solution timing with an alternating reference timing; store the
  reference duration and per-pair speed ratio with repeatability intervals.
- Advance the profile catalog to schema version 11 for paired reference metrics
  and bump the measurement harness to 0.12.0.
- Advance experiment capsules to schema version 13 with reference executable
  digests and compiler argv; replay and cross-host comparison verify both sides
  of paired timing.
- Migrate schema-12 capsules without inventing reference provenance, then
  regenerate and replay the checked-in Phase 1 capsule set under harness 0.13.0.
- Fixed Cachegrind's simulated cache geometry across hosts and bumped the
  measurement harness version to 0.9.0.
- Replace in-process allocator wrapping with an isolated Memcheck xtree pass for
  C, C++, and Rust; reject malformed or submission-forged reports, include
  allocation metrics in cross-machine checks, and bump the harness to 0.10.0.
- Make host metadata detection compile cleanly across Windows and macOS targets;
  enforce Rust warnings-as-errors in the CI matrix.

- Add a checked multi-file C++ sample with header, separate implementation,
  direct and CMake builds, hidden-case validation, and capsule replay coverage.
- Build Odin projects as package directories so sibling source files are
  compiled into the solution and emitted assembly.
- Add C multi-file direct and CMake examples with test, replay, and generator
  matrix coverage; split the Rust, D, and Zig samples across source modules.
- Add a Go multi-file sample and run its hidden suite through the pinned Go
  builder.
- Bound compiled-artifact transfer to 64 MiB, accept only regular artifact files, and test the exact size boundary, oversized files, symlinks, and FIFOs.
- Refresh the checked-in C++ comparison capsules against the current schema-12 toolchain environment and update their replay IDs and observed timing intervals.
- Refresh checked-in Rust, D, Zig, Odin, and Go capsules against the current schema-12 toolchain environment and verify deterministic replay for each language.
- Record the Go sample's harness-0.9.0 20-run instruction, branch, memory-reference, cache, and wall-clock spreads from the pinned runtime.
- Advance experiment capsules to schema version 12 with the measured executable SHA-256; reject replay when rebuilt bytes differ and require matching binary digests in the cross-machine gate.
- Advance the profile catalog to schema version 9 and list language coverage for each metric, including allocator metrics not instrumented by every runtime.
- Add `ccx experiment` to run public and hidden correctness tests before measurement, returning one combined test and capsule JSON report.
- Advance experiment capsules to schema version 11 with observed per-metric p95 and p99 sample quantiles; migrate older capsules without fabricating tail data.
- Include host, execution target, pinned toolchain environment, observed spread, and explicit uncertainty kinds in measurement JSON; preserve fractional ratios and IPC in tables.
- Advance capsules to schema version 10 with stable toolchain environment fingerprints, allowing replay across locally rebuilt images only when their pinned recipe and tool binaries match.
- Verify the measurement harness binary fingerprint separately in each GCC, Clang, and Rust image; expose stable environment fingerprints in the profile catalog schema version 8.
- Replay each of the three hosted repeatability capsules on the comparison runner after validating their shared environment fingerprint.
- Extend `ccx matrix` to compare direct, CMake with Ninja or Make, and bounded custom C/C++ builds on identical generated input.
- Make successful measurements fail clearly when Cachegrind cannot produce its required simulated counters; follow configured runtime wrappers while measuring.
- Apply the configured runtime argv to every batched correctness case, bump the harness version, and repin all execution images.
- Expose MSVC as an explicitly unavailable C/C++ profile until a pinned Windows runner is available.
- Advance the profile catalog to schema version 6 with explicit x86, x86-64, and ARM64 choices for Linux, Windows, and macOS.
- Advance capsules to schema version 9 and record runner-measured stdout and stderr byte totals.
- Expand Cachegrind reporting to separate instruction and data cache misses, reads, writes, branch instructions, and simulated miss rates.
- Bump the measurement harness version for the expanded result metrics.
- Advance the profile catalog to schema version 5 with exact IDs for the expanded metrics and custom command capabilities.
- Add custom C/C++ compiler argv templates and sandboxed runtime argv templates, both recorded and checked by replay.
- Add pinned GCC 13 and Clang 18 profiles alongside GCC 14 and Clang 19, selectable by configuration and comparable through the compiler matrix.
- Advance experiment capsules to schema version 8 with a version 7 migration for versioned compiler identities.
- Verify each compiler's exact version and binary fingerprint inside its pinned image with `ccx doctor`.
- Derive compiler, language, build-system, and Linux target availability from pinned toolchain and build-tool lock entries; advance the profile catalog to schema version 3 with per-choice availability and reasons.
- Let C and C++ projects choose the pinned CMake generator, Ninja or Unix Makefiles, and include the choice in configuration, build recipes, profile discovery, and `doctor` checks.
- Expose explicit OS and architecture selectors in `ccx init` and reject targets without a pinned execution profile.
- Limit selectable submission languages to C, C++, Rust, D, Zig, Odin, and Go while retaining generated assembly output through `ccx asm`.
- Report only executed cases in `ccx test` totals when a failing public case skips the requested generated sweep.
- Record the observed 1,000-seed generated-test throughput sample and its end-to-end limits.
- Add a versioned `ccx profiles` JSON catalog for language, compiler, build, target, and metric choices, including explicit unavailable profiles.
- Batch generated correctness cases inside pinned containers while retaining fresh per-case runner processes and sealed read-only input buffers.
- Kill leftover process-group descendants after each runner invocation, including programs that close stdout and stderr before hanging.
- Stop generated sweeps at the first failing case and report requested versus completed counts.
- Add `ccx matrix` to measure selected pinned compiler and optimization profiles into separate replayable capsules, verify shared input hashes, preserve results by capsule identity, and label pairwise differences within repeatability noise.
- Allow replay to locate a solution above a nested capsule path or accept an explicit `--dir` for externally stored capsules.
- Add per-case Linux process resource counters to correctness reports and JSONL logs.
- Isolate concurrent CLI pipeline invocations in private scratch directories and store successful build artifacts by binary hash.
- Build x86-64 assembly as assembly, omit unsupported optimization claims, and keep its generated source view per invocation.
- Add measured Linux process CPU time, page faults, context switches, and block-I/O operation counts to capsules.
- Add repeated clean-build wall and CPU timing to capsules and reject byte-different repeated builds.
- Add bounded parallel seeded sweeps of up to one million cases with streaming JSONL logs.
- Add explicit compiler version, target profile, standard/custom preset, and bounded C/C++ compiler flags to `ccx.toml` and `ccx init`.
- Refresh pinned image IDs and the harness hash for the resource-usage runner.
- Bump the harness version for repeated build measurements.
- Collect optional Linux hardware instructions, cache misses, branch misses, and CPU cycles with `perf_event_open`; derive host-specific IPC and effective average frequency, and fall back to simulated Cachegrind counts when unavailable.
- Make `ccx doctor` probe hardware counters inside the actual restricted container and report the active measurement methods.
- Bump the harness version and refresh all toolchain image IDs and harness hashes for hardware counter support.
- Allow bounded runtime argv customization through `ccx init` and `ccx.toml`, include it in experiment capsules, and reject replay when the configured arguments differ.
- Stage multi-file projects into isolated read-only build inputs so nested includes and private solution-directory permissions work with the non-root sandbox.
- Advance the profile catalog to schema version 2 for its runtime-argument capability.
- Add pinned Go support for build, test, measurement, disassembly, and replay.
- Advance experiment capsules to schema version 7 with Go migration.
- Bump the harness version for interval-based replay and repeated replay samples.

- Add pinned Zig 0.17.0 and Odin dev-2026-10 build, test, measurement,
  assembly, and replay support.
- Add Zig and Odin sorted pair sum solutions to the language validation matrix.
- Advance experiment capsules to schema version 6 with Zig and Odin migrations.

- Added D builds using pinned GDC 14.2.0 and schema version 4.
- Added a concurrent allocator instrumentation regression suite.

- Added repeated Cachegrind samples to `ccx measure` and median-based reported
  metric values.
- Added indicative wall-clock samples and seeded confidence intervals to
  capsules.
- Added recursive project input hashing and nested source discovery.
- Added explicit GCC or Clang selection to `ccx init`.
- Added initial trust methodology, authoring, quickstart, and limitations docs.
