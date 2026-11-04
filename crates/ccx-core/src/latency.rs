//! Latency distribution statistics.
//!
//! Wall-clock samples are always `indicative`: they move with the machine and
//! the load. This module turns a set of samples into the percentiles and
//! histogram a reader expects, and is deliberately conservative about what it
//! will claim: a percentile the sample count cannot support is reported as
//! *unavailable*, never interpolated into existence.
//!
//! The samples here are whole-process wall times, one per fresh run. That is a
//! *run-latency* distribution, not a per-request service time, and it does not
//! suffer coordinated omission the way a closed-loop client benchmark does. The
//! caller is expected to say so; the numbers must not be dressed up as a
//! server's tail latency.

use serde::{Deserialize, Serialize};

/// One bar of the histogram. `lower_ns` is inclusive, `upper_ns` exclusive;
/// the last bucket is open-ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub lower_ns: u64,
    pub upper_ns: Option<u64>,
    pub count: u64,
}

/// A requested percentile and its value, if the sample count supports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Percentile {
    /// The requested level, e.g. `0.99` for p99.
    pub level: f64,
    pub label: String,
    /// `None` when there are too few samples to resolve this percentile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_ns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencySummary {
    pub samples: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub mean_ns: f64,
    pub stdev_ns: f64,
    pub percentiles: Vec<Percentile>,
    pub histogram: Vec<HistogramBucket>,
    pub notes: Vec<String>,
}

/// The smallest sample count that can resolve a percentile at `level`.
///
/// To say "p99.9" honestly you need enough samples that at least one lies in
/// the top 0.1%, so `n * (1 - level) >= 1`. Below that the percentile would be
/// the maximum in disguise, which is a different (and misleading) claim.
fn supports(level: f64, samples: u64) -> bool {
    if !(0.0..1.0).contains(&level) {
        return false;
    }
    (samples as f64) * (1.0 - level) >= 1.0
}

/// Nearest-rank percentile over sorted samples. Returns `None` when the sample
/// count cannot resolve the level.
fn percentile_value(sorted: &[u64], level: f64) -> Option<u64> {
    if sorted.is_empty() || !supports(level, sorted.len() as u64) {
        return None;
    }
    let rank = (level * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[index])
}

/// Build a histogram with power-of-two edges, plus a zero bucket when any
/// sample is exactly zero.
fn histogram(samples: &[u64]) -> Vec<HistogramBucket> {
    let lower = samples[0].max(1);
    let mut buckets: Vec<HistogramBucket> = Vec::new();
    let mut edge = lower;
    while edge < u64::MAX / 2 {
        let upper = edge.saturating_mul(2);
        buckets.push(HistogramBucket {
            lower_ns: edge,
            upper_ns: Some(upper),
            count: 0,
        });
        edge = upper;
    }
    let mut zeros = 0u64;
    for &sample in samples {
        if sample == 0 {
            zeros += 1;
            continue;
        }
        // The bucket whose lower edge is the largest power of two <= sample.
        let mut placed = false;
        for bucket in buckets.iter_mut() {
            let upper = bucket.upper_ns.unwrap_or(u64::MAX);
            if sample >= bucket.lower_ns && sample < upper {
                bucket.count += 1;
                placed = true;
                break;
            }
        }
        if !placed {
            if let Some(last) = buckets.last_mut() {
                last.count += 1;
            }
        }
    }
    let mut out = Vec::new();
    if zeros > 0 {
        out.push(HistogramBucket {
            lower_ns: 0,
            upper_ns: Some(lower),
            count: zeros,
        });
    }
    out.extend(buckets);
    out
}

/// Summarize a set of wall-clock samples (nanoseconds).
pub fn summarize(samples: &[u64], levels: &[(f64, String)]) -> LatencySummary {
    let mut notes = Vec::new();
    if samples.is_empty() {
        return LatencySummary {
            samples: 0,
            min_ns: 0,
            max_ns: 0,
            mean_ns: 0.0,
            stdev_ns: 0.0,
            percentiles: levels
                .iter()
                .map(|(level, label)| Percentile {
                    level: *level,
                    label: label.clone(),
                    value_ns: None,
                })
                .collect(),
            histogram: Vec::new(),
            notes: vec!["no samples were recorded".to_string()],
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len() as f64;
    let mean = sorted.iter().map(|&v| v as f64).sum::<f64>() / n;
    let variance = if sorted.len() > 1 {
        sorted
            .iter()
            .map(|&v| {
                let d = v as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (n - 1.0)
    } else {
        0.0
    };

    let percentiles: Vec<Percentile> = levels
        .iter()
        .map(|(level, label)| {
            let value = percentile_value(&sorted, *level);
            if value.is_none() {
                notes.push(format!(
                    "{label} needs at least {} samples; only {} were taken",
                    (1.0 / (1.0 - level)).ceil() as u64,
                    sorted.len()
                ));
            }
            Percentile {
                level: *level,
                label: label.clone(),
                value_ns: value,
            }
        })
        .collect();

    let histogram = histogram(&sorted);

    LatencySummary {
        samples: sorted.len() as u64,
        min_ns: sorted[0],
        max_ns: *sorted.last().unwrap(),
        mean_ns: mean,
        stdev_ns: variance.sqrt(),
        percentiles,
        histogram,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels() -> Vec<(f64, String)> {
        vec![
            (0.5, "p50".to_string()),
            (0.9, "p90".to_string()),
            (0.99, "p99".to_string()),
            (0.999, "p99.9".to_string()),
        ]
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        // 1..=100, so p50 = 50, p90 = 90, p99 = 99. p99.9 needs 1000 samples.
        let samples: Vec<u64> = (1..=100).collect();
        let summary = summarize(&samples, &levels());
        let value = |label: &str| {
            summary
                .percentiles
                .iter()
                .find(|p| p.label == label)
                .unwrap()
                .value_ns
        };
        assert_eq!(value("p50"), Some(50));
        assert_eq!(value("p90"), Some(90));
        assert_eq!(value("p99"), Some(99));
        assert_eq!(value("p99.9"), None);
        assert_eq!(summary.samples, 100);
        assert_eq!(summary.min_ns, 1);
        assert_eq!(summary.max_ns, 100);
        assert!(summary.notes.iter().any(|n| n.contains("p99.9")));
    }

    #[test]
    fn p999_resolves_with_a_thousand_samples() {
        let samples: Vec<u64> = (1..=1000).collect();
        let summary = summarize(&samples, &levels());
        let value = summary
            .percentiles
            .iter()
            .find(|p| p.label == "p99.9")
            .unwrap()
            .value_ns;
        assert_eq!(value, Some(999));
    }

    #[test]
    fn empty_samples_are_reported_not_invented() {
        let summary = summarize(&[], &levels());
        assert_eq!(summary.samples, 0);
        assert!(summary.percentiles.iter().all(|p| p.value_ns.is_none()));
        assert!(!summary.notes.is_empty());
    }

    #[test]
    fn histogram_counts_every_sample_once() {
        let samples = vec![0, 1, 2, 3, 4, 5, 100, 1000];
        let summary = summarize(&samples, &levels());
        let total: u64 = summary.histogram.iter().map(|b| b.count).sum();
        assert_eq!(total, samples.len() as u64);
        // The zero sample lands in its own bucket.
        assert_eq!(summary.histogram[0].lower_ns, 0);
        assert_eq!(summary.histogram[0].count, 1);
    }

    #[test]
    fn stdev_matches_hand_computation() {
        let samples = vec![2, 4, 4, 4, 5, 5, 7, 9];
        let summary = summarize(&samples, &levels());
        assert!((summary.mean_ns - 5.0).abs() < 1e-9);
        // Sample standard deviation of this set is ~2.138.
        assert!((summary.stdev_ns - 2.138_089_935_299_395).abs() < 1e-9);
    }

    #[test]
    fn a_single_sample_has_no_distribution_to_report() {
        let summary = summarize(&[42], &levels());
        assert_eq!(summary.stdev_ns, 0.0);
        assert_eq!(summary.mean_ns, 42.0);
        assert_eq!(summary.min_ns, 42);
        assert_eq!(summary.max_ns, 42);
        // One sample cannot resolve any percentile, so none is claimed.
        assert!(summary.percentiles.iter().all(|p| p.value_ns.is_none()));
        assert!(summary.notes.iter().any(|n| n.contains("p50")));
    }
}
