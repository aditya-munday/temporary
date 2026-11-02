# AGENTS.md — CodeCurrency

Working directory: `/workspace/project`
Remote: `https://github.com/aditya-munday/CodeCurrency-Runtime.git`
  (renamed from `CodeCurrency`; old URLs redirect)
Branch: `main` is the integration branch. The engine landed as PR #1
  (`f5fb476`), x86-64 assembly and the capability matrix as PR #2
  (`cd6ee37`), and statistical tie groups, the Pareto frontier,
  cache-hierarchy/TLB counters, and the host-noise canary as PR #3
  (`79ae23f`). All three are merged and green.
Source idea document: `/workspace/project/PLATFORM_IDEA.txt`
Task list for the current phase: `PHASE1_AGENT_PROMPT.txt` (untracked)
GitHub CLI: authenticated as `aditya-munday` (scopes: gist, read:org, repo, workflow)

## What this project is

CodeCurrency is a **performance arena for systems programmers** — a practice,
competition, and learning platform for low-level performance (speed, memory,
concurrency, networking, IO, parsing, kernels, hardware).

Where other sites stop:
- LeetCode asks "is your answer correct?"
- Codeforces asks "can you solve it fast under contest pressure?"
- highload.fun asks "who runs fastest?"

CodeCurrency asks: **"How fast is it, on what machine, measured how, and WHY is
it that fast?"**

## The soul of the product (non-negotiable principles)

1. **Explained, not just ranked** — every result shows instructions, cache and
   branch behaviour, assembly, and a plain-language reason next to the number.
2. **Honest** — numbers carry uncertainty (e.g. `812 ms ± 0.6%`), never a fake
   single score. Ties are admitted when differences are within noise.
3. **Replayable** — every run is an "experiment capsule" (code, compiler, flags,
   inputs, machine) that anyone can re-run and get the same answer. Results are
   citable evidence.
4. **Wide difficulty range** — from "first loop" (Level 1) to research-grade
   (Level 9) and hall of fame (Level 10).
5. **Low-level languages are first-class** — C, C++, Rust, Zig, Odin, D,
   assembly, and more.
6. **Community of people who care why things are fast.**

## What it must NEVER become

- A LeetCode clone with a faster timer.
- A site with one meaningless global score.
- A noisy benchmark site claiming fake precision.
- A free-for-all of incomparable configurations.
- An AI chatbot that gives answers without evidence.
- A site where cheating or copied problems go unnoticed.

## Audience

Students learning how computers work; competitive programmers past the
"Accepted" ceiling; backend/game/embedded/trading/infra engineers; compiler/OS/
kernel enthusiasts; systems + performance interview candidates; teachers needing
measurable reproducible labs; companies seeking evidence-based performance talent.

## Problem universe

**Difficulty ladder (uniform across every field):**

| Level | Name | Tier |
|---|---|---|
| 1 | Warm-up | Normal |
| 2 | Easy | Normal |
| 3 | Medium | Intermediate |
| 4 | Cache-aware | Intermediate |
| 5 | Beat the reference | Advanced |
| 6 | SIMD / lock-free | Advanced |
| 7 | Budget challenges (cache misses, allocations, memory, size) | Expert |
| 8 | Hardware-specific (AVX2 / AVX-512 / NEON) | Specialised |
| 9 | Research-grade | Specialised |
| 10 | Hall of fame | Legendary |

**Fields (each grows to dozens of problems):** foundations of speed; DSA
performance-style; memory management and allocators; concurrency and parallelism
(threads, atomics, lock-free, false sharing, thread pools, NUMA); parsing and
text (JSON, CSV, logs, UTF-8, tokenizers); IO and files (buffering, mmap, async
IO, batching); networking and protocols (HTTP parsing, epoll-style servers,
codecs, tail latency); serialization and compression; SIMD and vectorisation;
databases and indexing (B-tree, LSM, hash tables); compilers and interpreters
(lexers, VMs, dispatch tricks); OS/kernel engineering as safe simulations
(schedulers, page replacement, buddy/slab allocators, virtual memory and TLB,
filesystem images, syscall replay); low-latency systems (order books, ring
buffers, p99/p99.9); distributed systems in a deterministic simulator; assembly
and microarchitecture; security-minded performance (constant-time code, safe
parsing); real-world regressions; embedded-style constraints (tiny memory, no
heap); interview track for systems/performance roles.

**Problem flavours:** classic (minimise time); budget (under N cache misses, zero
allocations in hot loop); Pareto (speed vs memory vs binary size); golf (smallest
code/binary); latency (best p99 on recorded workload); scaling (speed-up curve
1→N threads); debug-the-slowness; predict-first; reverse (read assembly);
open-ended (community votes on elegance).

## Coding environment requirements

- Real browser editor: multi-file projects, themes, keybindings, build presets,
  custom test inputs.
- Assembly panel beside source with line-by-line mapping and compiler remarks
  (e.g. "loop not vectorised because...").
- Counter panel: instructions, cache misses, branch misses, memory.
- **Code never resets:** autosave on every pause, full version history with
  diffs, restore any version, every submission linked to its exact snapshot,
  continue the same draft on any device, "continue where you left off" on the
  home page, per-language drafts kept separately.
- Same environment downloadable as CLI and container so local results match the
  site.
- Languages: C, C++ (GCC and Clang), Rust, Zig, x86-64 assembly first; then Odin,
  D, Nim, Go, Ada, Fortran, ARM64 assembly as the community asks.
- Multiple compilers and optimisation levels in one matrix (`-O2` vs `-O3`,
  GCC vs Clang).

## Measurement: the trust layer

- Deterministic metrics first: instruction counts, simulated cache/branch
  misses, allocations, peak memory, binary size — identical on any machine, so
  fair, instant, and nearly free. **Default ranking signal.**
- Wall-clock time shown as a distribution with an interval, always labelled with
  its machine.
- Paired runs: your code timed alternately with a reference to cancel machine
  drift.
- Hidden, freshly generated test data per run so hard-coding answers fails.
- Statistical ties: within-noise results share a rank; a secondary metric breaks
  the tie.
- Rankings are per problem, per language, per toolchain, per metric — no fake
  global score.
- Honest labels: "measured", "simulated", "predicted".

## Competitive and daily life

Daily puzzle (short, mini-leaderboard, archive); weekly contest (fixed time, 3–4
problems, instant scoring on deterministic metrics, editorials next day); rated
contests with per-field rating plus overall; seasonal events and marathons (e.g.
"SIMD Week"); sponsored contests; virtual contests (replay an old contest against
the old board); team contests and college/country leagues; leaderboards per
problem/category/contest/country/college/friends and "most improved"; trending,
hardest this week, newest records, weekly top performers; streaks with freezes,
badges, weekly goals, levels; multi-problem quests (e.g. "Build a fast web server
in 8 steps"); favourites and shareable lists; explore page; library with filters;
study plans ("C++ performance in 30 days", "Concurrency from zero", "SIMD in two
weeks", "Kernel basics", "Systems interview in 4 weeks").

## Community and learning

Per-problem discussion (approaches, Q&A, editorial) with spoiler-safe code hidden
until solved; ask questions attaching your experiment capsule so helpers see
counters, and request a "performance review"; upvotes, accepted answers,
reputation, helper badges; interview corner; short lessons with runnable examples
and predict-first quizzes; "explain it like I'm new" / "like an engineer" toggles;
community-submitted problems with review and credit; verified performance
portfolio — a public profile of replayable results and counters that employers
can check.

## AI woven into the product (not a button)

Always tied to evidence:
- Result page: short "why this is slow/fast" with sentences linking to hotspots
  and counters.
- Diff page: "your change reduced instructions 38% and branch misses 70%, but
  introduced a new cache-miss hotspot at line 42".
- Editor: quiet, evidence-based nudges after a run (e.g. "allocation inside the
  hot loop at line 18").
- Hints that adapt to attempt count, never revealing the full answer too early.
- Recommendations ("your weak spot is memory layout — try these 3").
- Study plan that adjusts to pace and gaps.
- Q&A: surfaces similar answered questions before posting.
- Editorial helper: drafts explanations for humans to review.
- Disabled in rated contests so competition stays fair.
- Cheap by design: explanations cached per result; most advice from rule-based
  analysis of counters, with an LLM only to phrase it.

## Extra ideas (beyond the original list)

1. Experiment capsules as shareable "proof links" for blogs, papers, hiring.
2. Explain-the-diff engine: splits any speed change into instructions, IPC,
   memory, branches with line-level blame.
3. Adversarial input market: players submit inputs that make others slow;
   survivors join the hidden suite and a "robustness" leaderboard column.
4. Counter-budget challenges (cache-miss / allocation / working-set limits).
5. Predict-first mode with "mental model" points for accurate predictions.
6. Compiler time-machine across compiler versions.
7. Open observatory: public dashboard of compiler/library performance over time.
8. Reverse-engineering puzzles from assembly or binary behaviour.
9. "Beat the library" battles (sort, hash map, memcpy, JSON parser).
10. Real-regression challenges from actual slow patches in small OSS libraries.
11. Hardware tracks (SIMD for x86 vs Arm, separate boards).
12. Teaching mode: private classroom assignments with measured results.
13. Energy / efficiency tracks (instructions per joule estimates) — later.
14. Pair-optimise sessions with live counters.
15. Community code review on result pages.
16. Performance bounties with verified winners.
17. Replay timeline: watch a top solution evolve version by version.
18. Daily "spot the slowness" two-minute snippet.
19. Cross-language duels (same task in C++, Rust, Zig side by side).
20. Accessibility and volunteer translations (dark/light, keyboard-only).

## Cost-efficiency principles

- Deterministic metrics need no special hardware → the default ranking signal, so
  contests and the daily puzzle cost almost nothing to judge.
- Expensive precise wall-clock on dedicated hardware is a rare, limited, clearly
  labelled extra — never the core.
- Users run code locally with the same container; the site only verifies, so most
  compute happens on users' machines.
- Results stored as small text capsules, not heavy logs.
- AI explanations cached and mostly rule-based.
- Static pages, free hosting tiers, open-source tools carry most of the load.
- Free forever: problems, local runs, public leaderboards, discussion. Optional
  paid extras later (private classrooms, extra verified runs, sponsored contests).
- Heavy features (rentable hardware lab, tracing, big fleets) wait until the
  community or sponsors can pay.

## The target feeling

Open the site → streak is waiting. Today's puzzle takes ten minutes. Yesterday's
half-finished allocator resumes exactly where it was left. Submit, and instead of
"Accepted" the page shows the assembly, explains why the loop was not vectorised,
shows cache misses fell 70% since the last version, and where you stand among
people statistically tied with you. Post a question with your result link;
someone replies within an hour. Compete at the weekend; next month share a
replayable proof of your best result on your profile.

**A gym, a laboratory, and an arena for people who want to truly understand fast
code.**

## Working notes for agents

- `main` holds only an early scaffold. The real engine lives on
  `codex/phase1-engine` (draft PR #1); base new work on that branch.
- Keep every design decision aligned with the trust layer (deterministic metrics,
  uncertainty, replayability) and the "must never become" list.
- Prefer small, verifiable increments; keep results reproducible and citable.
- Do not push directly to `main`/`master`; use feature branches and PRs.
- Keep this file current at the end of every working session.

## Phase 1 status (Rust engine, `crates/`)

Phase 1 proves one claim: a single command builds, tests, measures, and explains
a solution, and produces a result anyone can replay to the same numbers. Eight
build languages are first-class: C, C++, x86-64 assembly, Rust, D, Zig, Odin,
and Go. Execution is pinned to Linux x86-64; other OS/arch profiles report
"unavailable". No website, accounts, or contests yet.

Assembly is a selectable submission language (`language = "asm"`,
`toolchain = "gcc"`), built statically from Linux syscalls with no libc
required. `arena/asm` solves `sorted-pair-sum`. It reports no allocation metric
and no compiler remarks; both are marked unavailable rather than zero. The
scaffold emits a `.file` directive so the object is byte-identical across clean
builds. Per-language coverage is published in `docs/LANGUAGES.md` and by
`ccx profiles` (aliases `ccx languages`, `ccx platforms`).

Capsules are at **schema 17** (schema 15 added raw repeatability observations;
schema 16 adds the optional `applied_tolerance` field; schema 17 adds the
optional `noise_canary` field). See `CAPSULE_SPEC.md`.

### Host-noise canary

`ccx measure` runs a fixed C workload (`docker/harness/noise_canary.c`, id
`ccx-canary-v1`) once before and once after the solution's own runs, and records
the drift in the capsule's `noise_canary` field (`stable` / `drifted` /
`unavailable`). It is advisory: it never changes a metric or a verdict, and a
canary that cannot run is recorded as `unavailable` with a reason. `ccx replay`
surfaces a `drifted` or `unavailable` canary from the original measurement as a
note. Adding `noise_canary.c` to `docker/harness/` does **not** change the
harness fingerprint — `scripts/pin-toolchains.sh` hashes only `runner.c`,
`batch_runner.c`, and `alloc_shim.c` plus the built binaries — so committed
capsules stay valid. It is compiled on demand in the solution's image, so every
image needs a C compiler (all three have one).

### Commands

- `ccx build` — compile a solution in the pinned toolchain image.
- `ccx test` — run the arena solution against every size in `problem.json`.
- `ccx measure --size S --seed N --repeat R` — produce a capsule.
- `ccx replay <capsule.json>` — rebuild, re-measure, and compare to the capsule.
- `ccx init`, `ccx asm`, `ccx diff`, `ccx matrix`, `ccx pareto`,
  `ccx experiment`, `ccx validate`, `ccx doctor` — see `ccx --help`.

### Ranking and trade-offs

- `ccx matrix` ranks rankable profiles into statistical tie groups
  (`ccx_core::ranking::rank_groups`). Overlapping 95% intervals share a rank;
  ties are transitive; ineligible profiles are excluded, not ranked last.
- `ccx pareto` shows the multi-objective frontier over deterministic axes
  (`ccx_core::ranking::pareto_frontier`). It refuses wall-clock or
  hardware-counter axes: a frontier over a noise axis draws noise as structure.

### Layer 1: measurement truth (the trust layer)

The rule is absolute: a **measured** metric and a **simulated** one must never be
confused, and a measurement that is not real must not be reported.

Kinds and methods currently emitted:

| metric | kind | method |
| --- | --- | --- |
| `instructions`, `cache_misses`, `branch_misses`, `memory_refs`, `branches` | simulated | cachegrind |
| `allocations`, `allocated_bytes` | measured | manual (alloc shim) |
| `peak_memory` | indicative | getrusage |
| `binary_size` | measured | stat |

Hard-won facts, each of which was a real bug:

1. **Allocation counting requires static libstdc++.** `-Wl,--wrap=malloc` only
   rewrites call sites in objects we link. With a *shared* `libstdc++`,
   `std::vector` allocates inside the shared library and the shim reports `0`.
   The build therefore links `-static-libstdc++ -static-libgcc`. This is a
   correctness requirement for the number, not a performance choice.
2. **Valgrind must not be run with `--quiet`.** `--quiet` suppresses the summary
   block the parser reads, so the ranked metric disappears entirely.
3. **The cachegrind pass runs once (first iteration only).** Its result must be
   carried forward into the capsule; building the capsule from the *last*
   measurement silently drops every simulated metric.
4. **`replay` must measure every metric the capsule records.** Omitting
   allocations or binary size reports a spurious `DIFF` for a stable metric.
5. **A bare `capsule.json` has an empty parent path.** `replay` resolves the
   solution directory from the capsule's parent, falling back to `.`.
6. **The measured container's environment is canonical, hostname included.**
   Docker assigns a random 12-hex hostname to each `docker run` and exports it
   as the `HOSTNAME` environment variable; glibc's `__tunables_init` walks the
   environment, so a random hostname is a real (if small) input to the measured
   loader work. `SandboxPolicy::argv` now passes `--hostname` so the measured
   environment is byte-identical everywhere. Verified safe: the recorded
   `build.env` is unchanged, so no golden moved. Note the value's *length*
   matters, not its content, and the pinned value matches Docker's default
   length.
7. **The CI cross-machine gate's residual drift is CPU-model-driven, not
   environmental, and affects Rust as well as D.** The gate failed on
   `rust` with `branch_instructions differs across machines: [1265912.0,
   1265912.0, 1265940.0]` and on `d-gdc` with the same +22/+28 shape. This is
   *not* the hostname: within a single runner all 20 runs (each with a
   different random hostname) are byte-identical, so the counts are
   environment-independent, and the runners agree exactly whenever they land on
   the same CPU model. The cause is work that runs before `main` and reads the
   host CPU directly. For D that is druntime's `core.cpuid` startup probe
   (`cpuid_initialization`, `getcacheinfoCPUID4`). For Rust it is CPU-feature
   (ifunc) dispatch in the runtime and libc. Verified locally: varying
   `GLIBC_TUNABLES=glibc.cpu.hwcaps=...` moves a Rust binary by ~180 branches
   and the C++ binary by ~405, so glibc's dispatch really is inside the counted
   region; no container setting removes it. C, C++, Zig, and Odin binaries do
   not drift on the hosted pool. **Decision (schema 16):** the drift is
   admitted as a bounded per-host offset through named tolerance policies
   (`d-gdc-druntime-cpuid`, `rust-libc-ifunc-dispatch`) at `1e-3` relative over
   the thirteen simulated Cachegrind metrics, rather than by weakening the
   exact-equality gate. The bound sits just above the largest observed offset
   (0.043% on Rust/C++ `branch_misses`). Raw samples must still be constant
   within each host, so this admits a per-host offset, not within-host noise.
   `ccx replay` applies the same recorded bound. See `CAPSULE_SPEC.md` §16 and
   `METHODOLOGY.md`.
8. **Editing the runner invalidates every committed capsule.** The runner's
   SHA-256 is folded into the toolchain `environment_fingerprint`, which is in
   turn folded into the pinned image digest, and `Capsule::same_environment()`
   compares that fingerprint on replay. So a one-line change to `runner.c` (for
   example adding the cache-hierarchy and TLB counters in C1) changes the
   fingerprint, and every capsule recorded under the old runner fails replay's
   environment check with `the pinned toolchain environment fingerprint differs
   from the capsule`. The container-engine CI job runs
   `ccx replay evidence/sorted-pair-sum-great.json`, so the whole job goes red.
   Fix: after any runner change, rerun `scripts/pin-toolchains.sh` and
   re-measure every committed evidence capsule on the pinned image; the
   deterministic counters are unchanged, only the indicative timing moves.
   `scripts/compare-capsules.py` enforces the lock/capsule fingerprint match in
   CI, so the lock and the capsules must move together.

### Known-honest results (not bugs)

- `ccx test` runs the arena solution at every size in `problem.json`
  (`naive`=800, `good`=20000, `great`=200000). The naive O(n²) solution
  legitimately times out at n=200000. That is the point of the problem, not a
  harness failure.
- `peak_memory` drifts between runs because it comes from `getrusage` and is
  machine-dependent. It is labelled *indicative*, and replay attributes drift to
  the machine rather than the solution.
- Go counter spread between fresh processes is wider than the ranking gate, so
  Go measurements require `--repeat 20` and stay UNRANKED until the spread is
  within the documented tolerance.

### Verified state

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace --release`: 208 tests pass (145 core + 63 CLI), 1 ignored.
- `python3 -m unittest scripts/test_compare_capsules.py`: 22 tests pass.
- `ccx measure` on the naive solution: all metrics present and correctly
  labelled.
- `ccx replay capsule.json`: exit 0, every deterministic metric matches exactly,
  only the indicative `peak_memory` drifts as expected.

### Environment note

The container's Rust toolchain and Docker images have been observed to vanish
after a host restart. If `cargo` is missing, reinstall with
`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
--profile minimal`. The toolchain images are rebuilt by
`scripts/pin-toolchains.sh`, which rewrites `docker/toolchains.lock.json`; that
lock file is embedded in the binary via `include_str!`, so rebuild after
regenerating it.
