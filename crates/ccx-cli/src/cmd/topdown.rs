//! `ccx topdown` — where the issue slots went, from the hardware PMU.
//!
//! This is the one command that answers "why is it slow" rather than "how slow
//! is it". It runs the solution under `perf stat --topdown` and reports the
//! four level-one buckets — retiring, bad speculation, front-end bound,
//! back-end bound — whose fractions sum to one.
//!
//! The numbers come from the hardware, so they are labelled `measured`, but
//! they are also machine-dependent and are never a ranking signal, exactly like
//! the flat hardware counters the capsule already carries. When the host's PMU
//! refuses the events, or does not expose the slot counters, the command says
//! so instead of printing zeros.

use crate::cmd::{self, Fixable};
use crate::context::{policy_for, Solution};
use crate::pipeline::Pipeline;
use crate::Format;
use ccx_core::topdown::TopDownReport;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct TopDownCommandReport {
    pub toolchain: String,
    pub size: String,
    pub seed: u64,
    /// Always `measured`: these come from the hardware PMU.
    pub kind: String,
    /// Whether the four buckets were produced.
    pub available: bool,
    /// Why the split is unavailable, when it is.
    pub unavailable_reason: Option<String>,
    pub topdown: Option<TopDownReport>,
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
            "top-down profiling is unavailable for {:?} solutions",
            sol.language
        ))
        .fix("the analysis needs a binary with a debug build; see docs/LANGUAGES.md")
        .into_error());
    };

    let policy = policy_for(&sol.problem);
    let input = pipeline.generate(size, seed)?;
    let topdown = pipeline.top_down_counters(&program, &input, &policy)?;

    let Some(topdown) = topdown else {
        let report = TopDownCommandReport {
            toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
            size: size.to_string(),
            seed,
            kind: "measured".to_string(),
            available: false,
            unavailable_reason: Some(
                "the pinned analyzer image does not contain perf, so hardware PMU \
                 counters cannot be read on this host"
                    .to_string(),
            ),
            topdown: None,
            warnings,
        };
        return emit(format, &report);
    };

    if topdown.breakdown.is_none() {
        warnings.push(
            "the PMU did not yield a split; this is a host limitation, not a \
             property of the solution"
                .to_string(),
        );
    }

    let report = TopDownCommandReport {
        toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
        size: size.to_string(),
        seed,
        kind: "measured".to_string(),
        available: topdown.breakdown.is_some(),
        unavailable_reason: topdown.unavailable_reason.clone(),
        topdown: Some(topdown),
        warnings,
    };
    emit(format, &report)
}

fn emit(format: Format, report: &TopDownCommandReport) -> anyhow::Result<()> {
    cmd::emit(
        format,
        || {
            println!(
                "top-down microarchitecture ({}, size={}, seed={}, kind={})",
                report.toolchain, report.size, report.seed, report.kind
            );
            let Some(topdown) = report.topdown.as_ref() else {
                println!(
                    "  unavailable: {}",
                    report
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("no hardware counters on this host")
                );
                println!(
                    "  note: this is a measurement; it is machine-dependent and never a \
                     ranking signal"
                );
                return;
            };
            match &topdown.breakdown {
                Some(breakdown) => {
                    let bar = |fraction: f64| {
                        let filled = (fraction * 20.0).round().clamp(0.0, 20.0) as usize;
                        format!("{}{}", "#".repeat(filled), ".".repeat(20 - filled))
                    };
                    println!(
                        "  retiring        {:>6.1}%  {}",
                        breakdown.retiring * 100.0,
                        bar(breakdown.retiring)
                    );
                    println!(
                        "  bad speculation {:>6.1}%  {}",
                        breakdown.bad_speculation * 100.0,
                        bar(breakdown.bad_speculation)
                    );
                    println!(
                        "  front-end bound {:>6.1}%  {}",
                        breakdown.frontend_bound * 100.0,
                        bar(breakdown.frontend_bound)
                    );
                    println!(
                        "  back-end bound  {:>6.1}%  {}",
                        breakdown.backend_bound * 100.0,
                        bar(breakdown.backend_bound)
                    );
                    println!("  buckets sum      {:>5.1}%", breakdown.total() * 100.0);
                    let (bucket, share) = breakdown.dominant();
                    println!("  dominant: {bucket} ({:.0}%)", share * 100.0);
                }
                None => {
                    println!(
                        "  unavailable: {}",
                        report
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("the PMU did not expose the slot counters")
                    );
                }
            }
            if !topdown.raw_metrics.is_empty() {
                println!("\nraw perf metrics");
                for (bucket, value) in &topdown.raw_metrics {
                    println!("  {bucket:<18} {value}");
                }
            }
            for note in &topdown.notes {
                println!("\nnote: {note}");
            }
            for warning in &report.warnings {
                println!("\nwarning: {warning}");
            }
        },
        report,
    )
}
