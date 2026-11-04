//! `ccx latency` — the wall-clock distribution behind a capsule.
//!
//! A capsule records the median and spread of its indicative wall-clock
//! samples. This view adds the distribution those summaries hide: the
//! percentiles and the histogram a reader needs to see the tail.
//!
//! Everything here is `indicative`. These are whole-process run times, one per
//! fresh process, so they are a *run-latency* distribution, not a per-request
//! service time, and they move with the machine and its load. A percentile the
//! sample count cannot support is reported as unavailable rather than
//! interpolated into existence.

use crate::cmd::{self, Fixable};
use crate::Format;
use ccx_core::capsule::Capsule;
use ccx_core::latency::{self, LatencySummary};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The percentiles reported by default. `p99.9` needs a thousand samples, so on
/// a twenty-run capsule it is honestly reported as unavailable.
const DEFAULT_LEVELS: &[(f64, &str)] = &[
    (0.50, "p50"),
    (0.90, "p90"),
    (0.95, "p95"),
    (0.99, "p99"),
    (0.999, "p99.9"),
];

/// The unit a duration metric carries; only these can be summarized as latency.
const DURATION_UNIT: &str = "nanoseconds";

#[derive(Debug, Clone, Serialize)]
pub struct CapsuleLatency {
    pub capsule: String,
    pub capsule_id: Option<String>,
    pub language: String,
    pub metric: String,
    pub summary: LatencySummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyReport {
    pub metric: String,
    /// Always `indicative`: run latency is a wall-clock measurement.
    pub kind: String,
    /// What the distribution is, and is not, a claim about.
    pub interpretation: String,
    pub capsules: Vec<CapsuleLatency>,
    pub warnings: Vec<String>,
}

/// Parse `--level` values into `(level, label)` pairs.
///
/// A level must be strictly inside `(0, 1)`; `1.0` would be "the maximum in
/// disguise" and `0.0` the minimum, and neither is a percentile.
pub fn parse_levels(levels: &[String]) -> Result<Vec<(f64, String)>, String> {
    let mut parsed = Vec::new();
    for level in levels {
        let value: f64 = level
            .trim()
            .parse()
            .map_err(|_| format!("{level:?} is not a number"))?;
        if !(value > 0.0 && value < 1.0) {
            return Err(format!(
                "percentile level {value} is outside the open interval (0, 1)"
            ));
        }
        let label = format!("p{}", trim_level(value));
        parsed.push((value, label));
    }
    Ok(parsed)
}

/// `0.999` -> `"99.9"`, `0.5` -> `"50"`: a compact label for a percentile level.
fn trim_level(level: f64) -> String {
    let hundred = level * 100.0;
    let text = format!("{hundred:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

fn default_levels() -> Vec<(f64, String)> {
    DEFAULT_LEVELS
        .iter()
        .map(|(level, label)| (*level, (*label).to_string()))
        .collect()
}

fn load(path: &Path) -> anyhow::Result<Capsule> {
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

/// The raw samples a capsule recorded for `metric`, if it recorded any.
fn raw_samples(capsule: &Capsule, metric: &str) -> Option<Vec<f64>> {
    capsule
        .repeatability
        .as_ref()
        .and_then(|repeat| repeat.per_metric.get(metric))
        .and_then(|spread| spread.values.clone())
}

fn summarize(
    path: &Path,
    capsule: &Capsule,
    metric: &str,
    levels: &[(f64, String)],
) -> anyhow::Result<CapsuleLatency> {
    let Some(entry) = capsule.metrics.iter().find(|m| m.name == metric) else {
        return Err(
            Fixable::new(format!("{} has no metric named {metric:?}", path.display()))
                .fix("run `ccx latency <capsule>` with the default wall_clock metric")
                .fix("or pass --metric with a duration metric the capsule recorded")
                .into_error(),
        );
    };
    if entry.unit != DURATION_UNIT {
        return Err(Fixable::new(format!(
            "metric {metric:?} is measured in {}, not durations",
            entry.unit
        ))
        .fix("latency percentiles describe wall-clock durations in nanoseconds")
        .into_error());
    }
    let Some(values) = raw_samples(capsule, metric) else {
        return Err(Fixable::new(format!(
            "{} does not record raw samples for {metric:?}",
            path.display()
        ))
        .fix("re-measure with the current harness so the capsule keeps its observations")
        .fix("older capsules stored only the median and spread, not the samples")
        .into_error());
    };

    // A wall-clock sample is a whole nanosecond; the recorded f64 values are
    // integral. Rounding rather than truncating keeps a 1999.6 ns sample at
    // 2000 ns instead of silently shaving it.
    let samples: Vec<u64> = values
        .iter()
        .map(|value| value.round().max(0.0) as u64)
        .collect();

    Ok(CapsuleLatency {
        capsule: path.display().to_string(),
        capsule_id: capsule.capsule_id.clone(),
        language: format!("{:?}", capsule.source.language).to_lowercase(),
        metric: metric.to_string(),
        summary: latency::summarize(&samples, levels),
    })
}

pub fn run(
    format: Format,
    capsules: &[PathBuf],
    metric: &str,
    levels: &[String],
) -> anyhow::Result<()> {
    if capsules.is_empty() {
        return Err(Fixable::new("no capsules given")
            .fix("pass one or more capsules, e.g. `ccx latency arena/naive/capsule.json`")
            .into_error());
    }

    let levels = if levels.is_empty() {
        default_levels()
    } else {
        parse_levels(levels).map_err(|e| {
            Fixable::new(e)
                .fix("percentile levels are fractions, e.g. --level 0.99")
                .into_error()
        })?
    };

    let mut results = Vec::with_capacity(capsules.len());
    for path in capsules {
        let capsule = load(path)?;
        results.push(summarize(path, &capsule, metric, &levels)?);
    }

    let mut warnings = Vec::new();
    for result in &results {
        if result.summary.samples < 100 {
            warnings.push(format!(
                "{}: only {} samples; tail percentiles above p99 need at least 100",
                result.capsule, result.summary.samples
            ));
        }
    }

    let report = LatencyReport {
        metric: metric.to_string(),
        kind: "indicative".to_string(),
        interpretation: "run latency: whole-process wall time per fresh run, not per-request service time; it moves with the host and its load".to_string(),
        capsules: results,
        warnings,
    };

    cmd::emit(
        format,
        || {
            println!("latency | {} (indicative)", report.metric);
            println!("  {}", report.interpretation);
            for result in &report.capsules {
                let summary = &result.summary;
                println!();
                println!(
                    "  {} [{}] n={}",
                    result
                        .capsule_id
                        .as_deref()
                        .map(|id| &id[..12.min(id.len())])
                        .unwrap_or(&result.capsule),
                    result.language,
                    summary.samples,
                );
                println!(
                    "    min {}  median {}  mean {}  max {}  stdev {}",
                    fmt_ns(summary.min_ns),
                    fmt_ns(median_of(summary)),
                    fmt_ns(summary.mean_ns as u64),
                    fmt_ns(summary.max_ns),
                    fmt_ns(summary.stdev_ns as u64),
                );
                let percentiles: Vec<String> = summary
                    .percentiles
                    .iter()
                    .map(|p| match p.value_ns {
                        Some(value) => format!("{}={}", p.label, fmt_ns(value)),
                        None => format!("{}=unavailable", p.label),
                    })
                    .collect();
                println!("    {}", percentiles.join("  "));
                for note in &summary.notes {
                    println!("    note: {note}");
                }
                if !summary.histogram.is_empty() {
                    let peak = summary
                        .histogram
                        .iter()
                        .map(|bucket| bucket.count)
                        .max()
                        .unwrap_or(1)
                        .max(1);
                    println!("    histogram (log2 buckets):");
                    for bucket in &summary.histogram {
                        if bucket.count == 0 {
                            continue;
                        }
                        let bar = "#".repeat(((bucket.count * 24 / peak).max(1)) as usize);
                        let range = match bucket.upper_ns {
                            Some(upper) => {
                                format!("[{}, {})", fmt_ns(bucket.lower_ns), fmt_ns(upper))
                            }
                            None => format!("[{}, inf)", fmt_ns(bucket.lower_ns)),
                        };
                        println!("      {:<22} {:>4}  {}", range, bucket.count, bar);
                    }
                }
            }
            if !report.warnings.is_empty() {
                println!();
                for warning in &report.warnings {
                    println!("  warning: {warning}");
                }
            }
        },
        &report,
    )?;

    Ok(())
}

/// The p50 value when present, otherwise the mean rounded, for the summary line.
fn median_of(summary: &LatencySummary) -> u64 {
    summary
        .percentiles
        .iter()
        .find(|p| (p.level - 0.5).abs() < f64::EPSILON)
        .and_then(|p| p.value_ns)
        .unwrap_or(summary.mean_ns as u64)
}

/// Render nanoseconds in the largest unit that keeps the number readable.
fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.3}s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.3}ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.3}us", ns as f64 / 1e3)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_parse_and_label_compactly() {
        let levels = parse_levels(&["0.5".into(), "0.999".into()]).unwrap();
        assert_eq!(levels[0], (0.5, "p50".to_string()));
        assert_eq!(levels[1], (0.999, "p99.9".to_string()));
    }

    #[test]
    fn levels_outside_the_open_interval_are_rejected() {
        assert!(parse_levels(&["1.0".into()]).is_err());
        assert!(parse_levels(&["0".into()]).is_err());
        assert!(parse_levels(&["-0.5".into()]).is_err());
        assert!(parse_levels(&["abc".into()]).is_err());
    }

    #[test]
    fn default_levels_leave_tail_percentiles_unavailable_at_twenty_samples() {
        // Twenty samples can resolve p95 but not p99; the summary must say so
        // rather than invent a p99 from the maximum.
        let samples: Vec<u64> = (1..=20).map(|i| i * 1_000_000).collect();
        let summary = latency::summarize(&samples, &default_levels());
        let p50 = summary
            .percentiles
            .iter()
            .find(|p| p.label == "p50")
            .unwrap();
        let p99 = summary
            .percentiles
            .iter()
            .find(|p| p.label == "p99")
            .unwrap();
        assert_eq!(p50.value_ns, Some(10_000_000));
        assert_eq!(p99.value_ns, None);
        assert!(summary.notes.iter().any(|n| n.contains("p99")));
    }

    #[test]
    fn nanoseconds_render_in_the_largest_readable_unit() {
        assert_eq!(fmt_ns(999), "999ns");
        assert_eq!(fmt_ns(1_500), "1.500us");
        assert_eq!(fmt_ns(2_500_000), "2.500ms");
        assert_eq!(fmt_ns(3_000_000_000), "3.000s");
    }
}
