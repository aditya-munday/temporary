//! `ccx pareto` — the trade-off frontier across capsules.
//!
//! A single rank hides trade-offs. This view answers "what am I giving up":
//! among the capsules you pass in, which ones are not beaten on every chosen
//! axis at once? A capsule is dominated when another capsule is at least as
//! good on every axis and strictly better on one. The survivors are the
//! Pareto frontier.
//!
//! Only deterministic metrics belong on the axes. A machine-dependent metric
//! (wall-clock, hardware counters) is refused rather than drawn, because a
//! frontier over a noisy axis is a drawing of noise.

use crate::cmd::{self, Fixable};
use crate::Format;
use ccx_core::capsule::Capsule;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ParetoPoint {
    pub capsule: String,
    pub capsule_id: Option<String>,
    pub language: String,
    pub toolchain: String,
    /// Axis name -> value. Every value is `lower is better`.
    pub axes: std::collections::BTreeMap<String, f64>,
    pub dominated: bool,
    /// The capsule that dominates this one, when there is exactly one. When
    /// several capsules together dominate it, this is the first in input order.
    pub dominated_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParetoReport {
    pub axes: Vec<String>,
    pub points: Vec<ParetoPoint>,
    /// The frontier: points that nothing else beats on every axis at once.
    pub frontier: Vec<String>,
    /// Axis names requested but not present in any capsule.
    pub missing_axes: Vec<String>,
    pub warnings: Vec<String>,
}

fn load(path: &std::path::Path) -> anyhow::Result<Capsule> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        Fixable::new(format!("cannot read {}: {e}", path.display()))
            .fix("pass one or more capsule.json paths")
            .into_error()
    })?;
    Capsule::from_json(&text).map_err(|e| {
        Fixable::new(format!("{} is not a valid capsule: {e}", path.display()))
            .fix("regenerate it with `ccx measure`")
            .into_error()
    })
}

/// The default axes: the deterministic trade-offs Phase 1 promises.
const DEFAULT_AXES: &[&str] = &["instructions", "peak_memory", "binary_size"];

fn metric_value(capsule: &Capsule, name: &str) -> Option<f64> {
    capsule
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.value)
}

fn axis_is_deterministic(capsule: &Capsule, name: &str) -> Option<bool> {
    capsule
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.rankable_kind() && !metric.machine_dependent())
}

pub fn run(format: Format, capsules: &[std::path::PathBuf], axes: &[String]) -> anyhow::Result<()> {
    if capsules.is_empty() {
        return Err(Fixable::new("no capsules given")
            .fix("pass at least two capsules, e.g. `ccx pareto a/capsule.json b/capsule.json`")
            .into_error());
    }

    let loaded: Vec<(std::path::PathBuf, Capsule)> = capsules
        .iter()
        .map(|path| load(path).map(|capsule| (path.clone(), capsule)))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let axis_names: Vec<String> = if axes.is_empty() {
        DEFAULT_AXES.iter().map(|s| s.to_string()).collect()
    } else {
        axes.to_vec()
    };

    // An axis whose metric is machine-dependent is a noise axis; refuse it.
    let mut warnings = Vec::new();
    for name in &axis_names {
        for (path, capsule) in &loaded {
            if let Some(deterministic) = axis_is_deterministic(capsule, name) {
                if !deterministic {
                    return Err(Fixable::new(format!(
                        "axis {name:?} is machine-dependent in {}",
                        path.display()
                    ))
                    .fix("Pareto axes must be deterministic; use instructions, cache_misses, peak_memory, or binary_size")
                    .fix("drop the wall-clock or hardware-counter axis")
                    .into_error());
                }
            }
        }
    }

    let mut missing_axes = Vec::new();
    for name in &axis_names {
        if !loaded
            .iter()
            .any(|(_, capsule)| metric_value(capsule, name).is_some())
        {
            missing_axes.push(name.clone());
        }
    }
    if !missing_axes.is_empty() {
        warnings.push(format!(
            "axes not present in any capsule: {}",
            missing_axes.join(", ")
        ));
    }

    let mut points: Vec<ParetoPoint> = Vec::new();
    for (path, capsule) in &loaded {
        let mut point_axes = std::collections::BTreeMap::new();
        for name in &axis_names {
            if let Some(value) = metric_value(capsule, name) {
                point_axes.insert(name.clone(), value);
            }
        }
        points.push(ParetoPoint {
            capsule: path.display().to_string(),
            capsule_id: capsule.capsule_id.clone(),
            language: format!("{:?}", capsule.source.language).to_lowercase(),
            toolchain: capsule.toolchain.name.profile_id().to_string(),
            axes: point_axes,
            dominated: false,
            dominated_by: None,
        });
    }

    // A capsule with fewer axes than requested cannot be fairly compared on
    // the full axis set; the library keeps it off the frontier and flags it.
    let full_axes: Vec<String> = axis_names
        .iter()
        .filter(|name| !missing_axes.contains(name))
        .cloned()
        .collect();

    let entries: Vec<ccx_core::ranking::ParetoEntry> = points
        .iter()
        .map(|point| ccx_core::ranking::ParetoEntry {
            label: point.capsule.clone(),
            axes: point.axes.clone(),
        })
        .collect();
    let result = ccx_core::ranking::pareto_frontier(&entries, &full_axes);

    for point in &mut points {
        if let Some(dominator) = result.dominated_by.get(&point.capsule) {
            point.dominated = true;
            point.dominated_by = Some(dominator.clone());
        }
    }
    let frontier = result.frontier;

    let report = ParetoReport {
        axes: axis_names.clone(),
        points,
        frontier,
        missing_axes,
        warnings,
    };

    cmd::emit(
        format,
        || {
            println!("Pareto frontier over {}", report.axes.join(", "));
            println!("(every axis is lower-is-better)\n");
            for point in &report.points {
                let values: Vec<String> = report
                    .axes
                    .iter()
                    .map(|name| match point.axes.get(name) {
                        Some(value) => format!("{name}={value:.6e}"),
                        None => format!("{name}=unavailable"),
                    })
                    .collect();
                let status = if point.dominated {
                    format!(
                        "dominated by {}",
                        point.dominated_by.as_deref().unwrap_or("another capsule")
                    )
                } else if point.axes.len() == full_axes.len() {
                    "frontier".to_string()
                } else {
                    "incomplete (missing an axis)".to_string()
                };
                println!(
                    "  {} [{} {}] {} -> {}",
                    point.capsule,
                    point.language,
                    point.toolchain,
                    values.join(" "),
                    status
                );
            }
            println!("\n  frontier: {}", report.frontier.len());
            for path in &report.frontier {
                println!("    {path}");
            }
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
        },
        &report,
    )
}
