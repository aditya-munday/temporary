# Language capability matrix

CodeCurrency treats every supported language as a first-class submission
language. "First-class" does not mean "identical": each profile exposes the
measurements its toolchain can produce honestly, and every capability it cannot
produce is reported as `unavailable` with a reason. A missing capability is
never shown as a zero.

The same matrix is emitted by the CLI:

```sh
ccx profiles            # also `ccx languages` and `ccx platforms`
ccx profiles --format json
```

The `capabilities` array in the JSON output is the machine-readable form of the
table below. The table and the CLI share one source of truth in
`crates/ccx-cli/src/cmd/profiles.rs`, so they cannot drift apart.

## Matrix

| Language | Toolchain | Instructions | Cache sim | Branch sim | Allocations | Peak memory | Binary size | Compiler remarks | Asm view | Repeatability |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| C | GCC 14.2.0 | yes | yes | yes | yes | yes | yes | yes | yes | exact |
| C++ | GCC 14.2.0 | yes | yes | yes | yes | yes | yes | yes | yes | exact |
| x86-64 assembly | GCC 14.2.0 (GAS) | yes | yes | yes | no | yes | yes | no | yes | exact |
| Rust | rustc 1.85.1 | yes | yes | yes | yes | yes | yes | no | yes | bounded |
| Odin | dev-2026-10-nightly | yes | yes | yes | no | yes | yes | no | yes | exact |
| Zig | 0.17.0 | yes | yes | yes | no | yes | yes | no | yes | exact |
| Go | go1.26.10 | yes | yes | yes | no | yes | yes | yes | yes | unranked |
| D | GDC 14.2.0 | yes | yes | yes | no | yes | yes | yes | yes | bounded |
| Nim | planned | no | no | no | no | no | no | no | no | unavailable |
| Ada | planned | no | no | no | no | no | no | no | no | unavailable |
| Fortran | planned | no | no | no | no | no | no | no | no | unavailable |
| ARM64 assembly | planned | no | no | no | no | no | no | no | no | unavailable |

C and C++ also select GCC 13.3.0, Clang 18.1.8, and Clang 19.1.7. The matrix
reports the default GCC 14.2.0 profile; the other compiler selections share the
same capability set and differ only in the remarks pass (Clang uses `-Rpass`,
GCC uses `-fopt-info`).

The `planned` rows are language slots named so a client can explain and disable
them. Every capability is `no`, `ccx init` rejects them with the reason, and the
status column reads `planned`. They are not claimed as supported: each needs a
pinned image, a runner entry, and a problem fixture before it can move to
`available`.

## Column definitions

- **Instructions** — deterministic instruction counts from Cachegrind
  (`instructions`, `data_refs`, `data_reads`, `data_writes`, `branches`). The
  ranking signal.
- **Cache sim** — the fixed CCX v1 I1/D1 cache model (`cache_misses`,
  `instruction_cache_misses`, `data_cache_misses`, `cache_miss_rate`).
- **Branch sim** — the fixed CCX v1 branch model (`branch_misses`,
  `branch_instructions`, `branch_miss_rate`).
- **Allocations** — Valgrind Memcheck xtree allocation counts
  (`allocations`, `allocated_bytes`) for the main process.
- **Peak memory** — `getrusage` peak RSS, reported as `indicative`.
- **Binary size** — filesystem `stat` of the built artifact, reported as
  `measured`.
- **Compiler remarks** — a source-filtered optimizer report surfaced through
  `ccx asm` (`remarks_status`, `remarks_method`, `remarks_explanation`).
- **Asm view** — source-mapped assembly or disassembly from `ccx asm`.
- **Repeatability** — see the grades below.

## Repeatability grades

- **exact** — the deterministic counters are identical across clean builds and
  fresh processes; the same-host spread is `0.00%`. C, C++, assembly, Odin, and
  Zig reach this grade on the recorded problems.
- **bounded** — a small, reproducible per-host offset exists that no container
  setting removes. The capsule records an `applied_tolerance` naming the bound,
  and cross-machine comparison relaxes exact equality only for the named
  simulated metrics, only up to that bound, and only when every host declares
  the same policy. Raw within-host samples and measured metrics stay exact.
  - `rust` — `rust-libc-ifunc-dispatch`: the Rust runtime and libc resolve
    CPU-feature (ifunc) variants before `main`.
  - `d` — `d-gdc-druntime-cpuid`: the D druntime reads `core.cpuid` at startup.
- **unranked** — the language cannot yet produce counters stable enough to
  rank. Results are still reported and replayable; they are simply not ranked.
  - `go` — the runtime scheduler and GC make simulated counters vary between
    fresh processes. On the current recorded problem the instruction spread was
    0.79% and the cache-miss spread 6.69%, both above the 0.01% ranking
    tolerance. Go builds fix `GOROOT`, module and toolchain downloads, cache
    paths, and record `GOMAXPROCS=1` with the runtime seed setting, but that is
    not enough to reach the ranking tolerance. See
    [METHODOLOGY.md](METHODOLOGY.md), [LIMITATIONS.md](LIMITATIONS.md), and [PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md).

## Unavailable capabilities and why

- **Allocations for assembly, Odin, Zig, Go, and D.** Allocation accounting
  uses a Valgrind Memcheck xtree profile of the main process. C, C++, and Rust
  link a wrapped allocator shim so their counts are real; the other profiles do
  not, so the metric is reported as `unavailable` rather than fabricated as
  zero.
- **Compiler remarks for assembly.** Handwritten assembly has no compiler
  optimization pass, so there is nothing to report.
- **Compiler remarks for Rust, Zig, and Odin.** These pinned profiles do not
  expose a stable optimization-remark interface. The selected toolchain is
  reported with the reason instead of an empty report.

## Adding a language

An extended language slot (Nim, Ada, Fortran, ARM64 assembly, and so on) follows
the same profile mechanism: add a pinned toolchain image and fingerprint, a
`Language` variant, a build/assembly/remarks arm in the pipeline, and a
capability row. If a capability cannot be produced, add the row with that
capability marked unavailable and a reason. The four slots above already have a
`planned` row; promoting one to `available` means adding the pinned image, the
runner support, and a problem fixture, then flipping the row's `status`. See
[CONTRIBUTING.md](../CONTRIBUTING.md).
