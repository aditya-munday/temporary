//! `ccx hotspot` — where the instructions go, by function and by source line.
//!
//! The counter says *what*; attribution says *where*. This runs the solution
//! under Callgrind once and reads back two views of the same profile: a flat
//! function/source table, and a call-graph folded-stack export that can be
//! drawn as a flame graph.
//!
//! Every number here is Cachegrind's fixed model, so the whole report is
//! labelled `simulated`. It decomposes the `instructions` metric; it is not a
//! hardware sample and must never be presented as one.

use crate::cmd::{self, Fixable};
use crate::context::{policy_for, Solution};
use crate::pipeline::Pipeline;
use crate::Format;
use ccx_core::hotspot::{parse_annotate, parse_callgrind, render_flame_svg, HotspotReport};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct HotspotCommandReport {
    pub toolchain: String,
    pub size: String,
    pub seed: u64,
    /// Always `simulated`: Cachegrind's fixed model.
    pub metric_kind: String,
    pub source: String,
    pub report: HotspotReport,
    /// Folded stacks (`frame;frame cost`) for a flame graph.
    pub folded: Vec<String>,
    pub flame_svg_path: Option<String>,
    pub warnings: Vec<String>,
}

pub fn run(
    format: Format,
    dir: &Path,
    size: &str,
    seed: u64,
    flame: Option<&Path>,
) -> anyhow::Result<()> {
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
            "hotspot attribution is unavailable for {:?} solutions",
            sol.language
        ))
        .fix("only C, C++, Rust, and D direct builds carry the debug info Callgrind needs")
        .fix("see docs/LANGUAGES.md for the per-language capability matrix")
        .into_error());
    };

    let policy = policy_for(&sol.problem);
    let input = pipeline.generate(size, seed)?;
    let (annotate_text, callgrind) = pipeline.profile_callgrind(&program, &input, &policy)?;

    let report = parse_annotate(&annotate_text);
    if report.program_total == 0 {
        warnings.push("the profile recorded no instruction events".to_string());
    }
    if report.sources.iter().all(|source| source.lines.is_empty()) {
        warnings.push(
            "no source lines were annotated; the profile may predate the source layout".to_string(),
        );
    }

    let mut folded: Vec<String> = Vec::new();
    let mut flame_svg_path = None;
    if let Some(text) = &callgrind {
        let graph = parse_callgrind(text);
        folded = graph
            .folded_stacks()
            .into_iter()
            .map(|(stack, cost)| format!("{stack} {cost}"))
            .collect();
        if let Some(path) = flame {
            let svg = render_flame_svg(
                &graph.folded_stacks(),
                "CodeCurrency hotspots (Cachegrind, simulated)",
            );
            std::fs::write(path, svg)?;
            flame_svg_path = Some(path.display().to_string());
        }
    } else {
        warnings.push("the raw Callgrind file was not carried out of the sandbox".to_string());
    }

    let report = HotspotCommandReport {
        toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
        size: size.to_string(),
        seed,
        metric_kind: "simulated".to_string(),
        source: "Cachegrind callgrind_annotate (fixed CCX v1 model)".to_string(),
        report,
        folded,
        flame_svg_path,
        warnings,
    };

    cmd::emit(
        format,
        || {
            println!(
                "hotspots ({}, size={}, seed={})",
                report.toolchain, report.size, report.seed
            );
            println!("  source: {} [{}]", report.source, report.metric_kind);
            if let Some(path) = &report.flame_svg_path {
                println!("  flame graph: {path}");
            }
            println!(
                "\nfunction hotspots (Ir = instructions, simulated)\n{:<14} {:<40} file",
                "Ir", "function"
            );
            for function in report.report.functions.iter().take(20) {
                println!(
                    "{:<14} {:<40} {}",
                    function.ir, function.function, function.file
                );
            }
            for source in &report.report.sources {
                let hot: Vec<_> = source.lines.iter().filter(|line| line.ir > 0).collect();
                if hot.is_empty() {
                    continue;
                }
                println!("\nhot lines: {}", source.source);
                for line in hot.iter().take(20) {
                    println!("{:>6} {:>14}  {}", line.line, line.ir, line.text);
                }
            }
            for warning in &report.warnings {
                println!("\nwarning: {warning}");
            }
        },
        &report,
    )
}
