//! `ccx memory` — the heap high-water mark and where the live bytes sit.
//!
//! Two numbers answer "how much memory did it use", and they are not the same
//! number:
//!
//! * **peak memory** from the kernel cgroup high-water mark when the host
//!   exposes one that covers the run, else `getrusage` peak RSS, labelled
//!   `indicative`. It counts pages the process touched, including allocator and
//!   loader reservations, and varies with the host's page size and the runtime's
//!   startup. The report names which source produced it.
//! * **peak live heap bytes** from Valgrind Massif, labelled `measured`. It is
//!   the maximum of the program's own live allocation bytes, deterministic for
//!   a fixed input, and decomposed by allocation site.
//!
//! This command reports both, plus the power-of-two size classes of the peak
//! snapshot's leaves, so a reader can see *where* the memory went, not just how
//! much there was.

use crate::cmd::{self, Fixable};
use crate::context::{policy_for, Solution};
use crate::pipeline::Pipeline;
use crate::Format;
use ccx_core::memory::{parse_massif, MassifProfile};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct MemoryCommandReport {
    pub toolchain: String,
    pub size: String,
    pub seed: u64,
    /// Always `indicative`: peak memory is host-dependent.
    pub peak_memory_kind: String,
    pub peak_memory_bytes: u64,
    /// `cgroup` when the kernel charged the run a high-water mark, else
    /// `getrusage`.
    pub peak_memory_source: String,
    /// Always `measured`: Massif's live-heap high-water mark.
    pub heap_kind: String,
    pub heap: Option<MassifProfile>,
    pub warnings: Vec<String>,
}

pub fn run(format: Format, dir: &Path, size: &str, seed: u64) -> anyhow::Result<()> {
    let sol = Solution::load(dir)?;
    let pipeline = Pipeline::new(&sol)?;
    let tc = sol.toolchain()?;
    sol.ensure_dirs()?;

    if !sol.problem.meta.sizes.contains_key(size) {
        return Err(Fixable::new(format!("problem has no size named {size:?}"))
            .fix(format!(
                "available sizes: {}",
                sol.problem
                    .meta
                    .sizes
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .into_error());
    }

    let mut warnings = Vec::new();
    let Some(program) = pipeline.build_solution_for_profiling()? else {
        return Err(Fixable::new(format!(
            "memory profiling is unavailable for {:?} solutions",
            sol.language
        ))
        .fix("only C, C++, Rust, and D direct builds carry the debug info Massif needs")
        .fix("see docs/LANGUAGES.md for the per-language capability matrix")
        .into_error());
    };

    let policy = policy_for(&sol.problem);
    let input = pipeline.generate(size, seed)?;

    // Peak memory from an ordinary run. The runner reports the kernel cgroup
    // high-water mark when the host exposes one that covers this run, else
    // `getrusage` peak RSS; the source travels with the number so the label is
    // never guessed.
    let outcome = pipeline.run_program(&program, &input, &policy, false)?;
    let peak_memory_bytes = outcome
        .resources
        .as_ref()
        .and_then(|resources| resources.peak_memory_bytes)
        .unwrap_or(outcome.peak_rss_bytes);
    let peak_memory_source = outcome
        .resources
        .as_ref()
        .and_then(|resources| resources.peak_memory_source.clone())
        .unwrap_or_else(|| "getrusage".to_string());

    let heap = match pipeline.profile_massif(&program, &input, &policy)? {
        Some(text) => match parse_massif(&text) {
            Some(profile) => Some(profile),
            None => {
                warnings.push(
                    "the Massif output could not be parsed; the live-heap high-water \
                     mark is unavailable for this run"
                        .to_string(),
                );
                None
            }
        },
        None => {
            warnings.push("the Massif profile was not carried out of the sandbox".to_string());
            None
        }
    };

    if let Some(heap) = &heap {
        if heap.peak_live_bytes == 0 {
            warnings.push(
                "the program allocated nothing on the heap; a zero high-water mark is a \
                 real result, not a missing one"
                    .to_string(),
            );
        }
    }

    let report = MemoryCommandReport {
        toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
        size: size.to_string(),
        seed,
        peak_memory_kind: "indicative".to_string(),
        peak_memory_bytes,
        peak_memory_source: peak_memory_source.clone(),
        heap_kind: "measured".to_string(),
        heap,
        warnings,
    };

    cmd::emit(
        format,
        || {
            println!(
                "memory profile ({}, size={}, seed={})",
                report.toolchain, report.size, report.seed
            );
            println!(
                "  peak memory:     {} B [{}] ({}; includes allocator and loader pages)",
                report.peak_memory_bytes, report.peak_memory_kind, report.peak_memory_source
            );
            if let Some(heap) = &report.heap {
                println!(
                    "  peak live heap:  {} B [{}] (Massif, the program's own live bytes)",
                    heap.peak_live_bytes, report.heap_kind
                );
                if !heap.size_classes.is_empty() {
                    println!("\nsize classes at the peak (bytes held by sites in each bucket)");
                    println!(
                        "{:<14} {:<14} {:>14} {:>7}",
                        "lower", "upper", "bytes", "sites"
                    );
                    for class in &heap.size_classes {
                        let upper = class
                            .upper_bytes
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "+".to_string());
                        println!(
                            "{:<14} {:<14} {:>14} {:>7}",
                            class.lower_bytes, upper, class.bytes, class.sites
                        );
                    }
                }
                if !heap.sites.is_empty() {
                    println!("\nallocation sites holding the peak live bytes");
                    println!("{:>14}  location", "bytes");
                    for site in heap.sites.iter().take(20) {
                        println!("{:>14}  {}", site.bytes, site.location);
                    }
                }
                for note in &heap.notes {
                    println!("\nnote: {note}");
                }
            }
            for warning in &report.warnings {
                println!("\nwarning: {warning}");
            }
        },
        &report,
    )
}
