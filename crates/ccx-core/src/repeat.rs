//! Reproducibility report.
//!
//! The ranking gate is a claim: a deterministic metric repeats within a
//! documented tolerance. This module turns that claim into a number that can be
//! read off one machine, so the tolerance quoted in `docs/METHODOLOGY.md` is
//! measured rather than asserted. It only ever looks at deterministic metrics;
//! indicative and getrusage counters are counted but never gated.

use crate::capsule::repeatability::median;
use crate::capsule::{Capsule, MetricKind};
use serde::Serialize;

/// One metric's achieved tolerance across the repeated measurements.
#[derive(Debug, Clone, Serialize)]
pub struct MetricReproducibility {
    pub name: String,
    pub kind: MetricKind,
    pub observations: usize,
    pub min: f64,
    pub max: f64,
    pub median: f64,
    /// `(max - min) / median * 100`, the same definition the capsule uses.
    pub spread_pct: f64,
    /// Every observation was identical.
    pub exact: bool,
    /// The spread is within the documented ranking tolerance.
    pub within_tolerance: bool,
}

/// The achieved tolerance of one `ccx repeat` session.
#[derive(Debug, Clone, Serialize)]
pub struct ReproducibilityReport {
    pub runs: usize,
    pub capsule_ids: Vec<String>,
    /// The tolerance the engine claims (from the ranking gate).
    pub tolerance_pct: f64,
    /// The worst deterministic spread actually observed.
    pub achieved_tolerance_pct: f64,
    /// Every deterministic metric repeated within the tolerance.
    pub all_deterministic_within_tolerance: bool,
    pub deterministic: Vec<MetricReproducibility>,
    /// Machine-dependent metrics were seen but are not gated.
    pub machine_dependent_metric_count: usize,
}

/// Summarize N capsules into a reproducibility report.
///
/// A metric is included only when it is deterministic *and* present in every
/// capsule, so the comparison is over a like-for-like set.
pub fn summarize(capsules: &[Capsule], tolerance_pct: f64) -> ReproducibilityReport {
    let runs = capsules.len();
    let capsule_ids = capsules
        .iter()
        .map(|c| c.capsule_id.clone().unwrap_or_default())
        .collect();

    let machine_dependent_metric_count = capsules
        .first()
        .map(|c| c.metrics.iter().filter(|m| m.machine_dependent()).count())
        .unwrap_or(0);

    let mut deterministic = Vec::new();
    if let Some(first) = capsules.first() {
        for metric in &first.metrics {
            if metric.machine_dependent() {
                continue;
            }
            let mut values = Vec::with_capacity(runs);
            let mut present_everywhere = true;
            for capsule in capsules {
                match capsule.metrics.iter().find(|m| m.name == metric.name) {
                    Some(found) if !found.machine_dependent() => values.push(found.value),
                    _ => {
                        present_everywhere = false;
                        break;
                    }
                }
            }
            if !present_everywhere || values.is_empty() {
                continue;
            }
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let med = median(&values).unwrap_or(0.0);
            let spread_pct = if med == 0.0 {
                if max == min {
                    0.0
                } else {
                    f64::INFINITY
                }
            } else {
                (max - min) / med.abs() * 100.0
            };
            deterministic.push(MetricReproducibility {
                name: metric.name.clone(),
                kind: metric.kind,
                observations: values.len(),
                min,
                max,
                median: med,
                spread_pct,
                exact: max == min,
                within_tolerance: spread_pct <= tolerance_pct,
            });
        }
    }

    deterministic.sort_by(|a, b| a.name.cmp(&b.name));
    let achieved_tolerance_pct = deterministic
        .iter()
        .map(|m| m.spread_pct)
        .fold(0.0_f64, f64::max);
    let all_deterministic_within_tolerance =
        !deterministic.is_empty() && deterministic.iter().all(|m| m.within_tolerance);

    ReproducibilityReport {
        runs,
        capsule_ids,
        tolerance_pct,
        achieved_tolerance_pct,
        all_deterministic_within_tolerance,
        deterministic,
        machine_dependent_metric_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capsule::{Metric, MetricKind, MetricMethod};

    fn metric(name: &str, value: f64, kind: MetricKind, method: MetricMethod) -> Metric {
        Metric {
            name: name.to_string(),
            value,
            unit: "count".to_string(),
            kind,
            method,
            uncertainty: None,
        }
    }

    fn capsule(id: &str, instructions: f64, wall: f64) -> Capsule {
        let mut c: Capsule =
            serde_json::from_str(include_str!("../tests/fixtures/capsule-v15.json"))
                .expect("fixture parses");
        c.capsule_id = Some(id.to_string());
        c.metrics = vec![
            metric(
                "instructions",
                instructions,
                MetricKind::Simulated,
                MetricMethod::Cachegrind,
            ),
            metric(
                "wall_clock",
                wall,
                MetricKind::Indicative,
                MetricMethod::WallClock,
            ),
        ];
        c
    }

    #[test]
    fn identical_deterministic_metrics_are_within_tolerance() {
        let capsules = vec![
            capsule("a", 1000.0, 10.0),
            capsule("b", 1000.0, 99.0),
            capsule("c", 1000.0, 5.0),
        ];
        let report = summarize(&capsules, 0.01);
        assert_eq!(report.runs, 3);
        assert!(report.all_deterministic_within_tolerance);
        assert_eq!(report.achieved_tolerance_pct, 0.0);
        // wall_clock is indicative and must not appear in the gated set.
        assert!(report.deterministic.iter().all(|m| m.name != "wall_clock"));
        assert_eq!(report.machine_dependent_metric_count, 1);
    }

    #[test]
    fn a_drifting_deterministic_metric_fails_the_gate() {
        let capsules = vec![
            capsule("a", 1000.0, 10.0),
            capsule("b", 1002.0, 10.0),
            capsule("c", 1000.0, 10.0),
        ];
        let report = summarize(&capsules, 0.01);
        let instructions = report
            .deterministic
            .iter()
            .find(|m| m.name == "instructions")
            .unwrap();
        assert!(!instructions.exact);
        assert!(!instructions.within_tolerance);
        assert!(report.achieved_tolerance_pct > 0.0);
        assert!(!report.all_deterministic_within_tolerance);
    }

    #[test]
    fn a_metric_missing_from_one_capsule_is_excluded() {
        let mut a = capsule("a", 1000.0, 10.0);
        let mut b = capsule("b", 1000.0, 10.0);
        a.metrics.retain(|m| m.name != "instructions");
        b.metrics.retain(|m| m.name != "instructions");
        let report = summarize(&[a, b], 0.01);
        assert!(report
            .deterministic
            .iter()
            .all(|m| m.name != "instructions"));
    }
}
