//! `ccx repeat` — measure the same solution N times and report the achieved
//! reproducibility.
//!
//! `ccx measure --repeat R` takes R samples inside one capsule. `ccx repeat`
//! instead runs `ccx measure` N separate times, each with its own capsule and
//! its own repeat count, and reports the spread *across* those capsules. That
//! is the stronger claim: it catches drift that a single session's samples
//! would hide, and it measures the tolerance the ranking gate actually claims.

use crate::cmd::{self, Fixable};
use crate::context::Solution;
use crate::Format;
use ccx_core::capsule::{repeatability, Capsule};
use ccx_core::repeat::summarize;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn run(
    format: Format,
    dir: &Path,
    repeat: u32,
    samples: u32,
    out: Option<&Path>,
    size: &str,
    seed: u64,
) -> anyhow::Result<()> {
    if samples < 2 {
        return Err(
            Fixable::new(format!("repeat count {samples} cannot establish a spread"))
                .fix("use at least 2 repeats; 20 or more matches the ranking gate")
                .into_error(),
        );
    }
    let sol = Solution::load(dir)?;

    let out_dir = match out {
        Some(path) => path.to_path_buf(),
        None => sol.root.join("repeat"),
    };
    std::fs::create_dir_all(&out_dir)?;

    let mut capsules = Vec::with_capacity(samples as usize);
    let mut paths = Vec::with_capacity(samples as usize);
    for index in 0..samples {
        let path = out_dir.join(format!("capsule-{index:03}.json"));
        // Each sample is a full, self-contained capsule on disk.
        let _ = super::measure::measure_solution(&sol, repeat, Some(&path), size, seed)?;
        let text = std::fs::read_to_string(&path)?;
        capsules.push(Capsule::from_json(&text)?);
        paths.push(path);
    }

    let report = summarize(
        &capsules,
        repeatability::DEFAULT_RANKING_SPREAD_TOLERANCE_PCT,
    );

    cmd::emit(
        format,
        || {
            println!(
                "reproducibility | {} repeats x {} samples = {} measurements",
                samples,
                repeat,
                samples as u64 * repeat as u64
            );
            println!(
                "  tolerance claimed  {:.4}% (ranking gate)",
                report.tolerance_pct
            );
            println!(
                "  tolerance achieved {:.4}% over {} deterministic metrics",
                report.achieved_tolerance_pct,
                report.deterministic.len()
            );
            println!(
                "  verdict            {}",
                if report.all_deterministic_within_tolerance {
                    "REPRODUCIBLE"
                } else {
                    "NOT REPRODUCIBLE"
                }
            );
            for metric in &report.deterministic {
                println!(
                    "  {:<28} {:<10} min {:<14} max {:<14} spread {:.4}%{}",
                    metric.name,
                    format!("{:?}", metric.kind).to_lowercase(),
                    fmt(metric.min),
                    fmt(metric.max),
                    metric.spread_pct,
                    if metric.exact { " (exact)" } else { "" }
                );
            }
            if report.machine_dependent_metric_count > 0 {
                println!(
                    "  {} machine-dependent metrics were recorded but not gated",
                    report.machine_dependent_metric_count
                );
            }
            println!("  capsules in {}", out_dir.display());
        },
        &report,
    )?;

    if !report.all_deterministic_within_tolerance {
        return Err(Fixable::new(format!(
            "the solution did not reproduce across {samples} repeats"
        ))
        .fix(format!(
            "worst deterministic spread was {:.4}%, above the {:.4}% ranking tolerance",
            report.achieved_tolerance_pct, report.tolerance_pct
        ))
        .fix("check for a background load or a host-noise canary drift, then repeat")
        .into_error());
    }
    Ok(())
}

fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}
