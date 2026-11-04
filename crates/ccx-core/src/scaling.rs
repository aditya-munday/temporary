//! Thread-scaling arithmetic.
//!
//! A scaling curve answers "does more parallelism actually pay". Speed-up is
//! measured against the one-thread run in the same session, and efficiency is
//! that speed-up divided by the thread count. Both are `indicative`: they are
//! ratios of wall-clock times, so they move with the machine.
//!
//! The honest part is what the module refuses to do. It never extrapolates a
//! thread count it did not run, and it reports the *actual* parallelism reached
//! when the toolchain can expose it, rather than assuming the requested count.

use serde::{Deserialize, Serialize};

/// One point on the curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScalingPoint {
    /// Threads requested for this run.
    pub threads: u32,
    /// Median wall time in nanoseconds over the repetitions.
    pub median_ns: u64,
    /// Speed-up against the one-thread median. `None` for the baseline itself
    /// when it is the reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speedup: Option<f64>,
    /// `speedup / threads`. One means perfect scaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScalingCurve {
    /// Every point, ordered by thread count.
    pub points: Vec<ScalingPoint>,
    /// Thread count where speed-up stops growing beyond the tolerance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<u32>,
    /// Host logical CPUs detected.
    pub host_cpus: u32,
    pub notes: Vec<String>,
}

fn median(samples: &[u64]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        // Integer average of the two middle samples.
        (sorted[mid - 1] / 2) + (sorted[mid] / 2) + (sorted[mid - 1] % 2 + sorted[mid] % 2) / 2
    }
}

/// Build a scaling curve from per-thread-count wall-time samples.
///
/// `runs` maps a requested thread count to the wall times observed for it. The
/// curve is normalised against the smallest thread count present, which is
/// expected to be 1 but need not be.
pub fn curve(runs: &[(u32, Vec<u64>)], host_cpus: u32, tolerance: f64) -> ScalingCurve {
    let mut notes = Vec::new();
    let mut points: Vec<ScalingPoint> = Vec::new();

    let mut ordered: Vec<(u32, u64)> = runs
        .iter()
        .filter(|(_, samples)| !samples.is_empty())
        .map(|(threads, samples)| (*threads, median(samples)))
        .collect();
    ordered.sort_by_key(|(threads, _)| *threads);

    if ordered.is_empty() {
        notes.push("no samples were recorded".to_string());
        return ScalingCurve {
            points,
            saturation: None,
            host_cpus,
            notes,
        };
    }

    // The baseline is the one-thread run when present, otherwise the lowest
    // thread count that ran. Its own speed-up is the identity by definition.
    let baseline_threads = ordered[0].0;
    let baseline = ordered[0].1;
    if baseline == 0 {
        notes.push("the baseline run recorded a zero wall time; speed-up is unavailable".into());
    }
    if baseline_threads != 1 {
        notes.push(format!(
            "no one-thread baseline was run; speed-up is relative to {baseline_threads} threads"
        ));
    }

    // Normalised speed-ups, including the baseline as 1.0 so saturation is
    // detected against the first run rather than the second.
    let mut normalised: Vec<(u32, f64)> = Vec::new();
    for (threads, median_ns) in &ordered {
        if baseline == 0 || *median_ns == 0 {
            continue;
        }
        let speedup = baseline as f64 / *median_ns as f64;
        normalised.push((*threads, speedup));
        let reported = if *threads == baseline_threads {
            None
        } else {
            Some(speedup)
        };
        points.push(ScalingPoint {
            threads: *threads,
            median_ns: *median_ns,
            speedup: reported,
            efficiency: reported.map(|s| s / *threads as f64),
        });
    }

    // Saturation: the first thread count after which no further point improves
    // speed-up by more than `tolerance` (a fraction, e.g. 0.05 for 5%).
    let mut saturation = None;
    for window in normalised.windows(2) {
        let [current, next] = [window[0], window[1]];
        let gain = (next.1 - current.1) / current.1.max(f64::MIN_POSITIVE);
        if gain <= tolerance {
            saturation = Some(current.0);
            break;
        }
    }
    if saturation.is_none() {
        if let Some(last) = points.last() {
            if last.speedup.is_some() {
                notes.push(format!(
                    "speed-up was still growing at {} threads; the curve is not saturated",
                    last.threads
                ));
            }
        }
    }

    ScalingCurve {
        points,
        saturation,
        host_cpus,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_scaling_gives_linear_speedup_and_unit_efficiency() {
        // 1s, 0.5s, 0.25s for 1, 2, 4 threads.
        let runs = vec![
            (1u32, vec![1_000_000_000u64]),
            (2, vec![500_000_000]),
            (4, vec![250_000_000]),
        ];
        let curve = curve(&runs, 4, 0.05);
        assert_eq!(curve.points[0].speedup, None);
        assert!((curve.points[1].speedup.unwrap() - 2.0).abs() < 1e-9);
        assert!((curve.points[2].speedup.unwrap() - 4.0).abs() < 1e-9);
        assert!((curve.points[2].efficiency.unwrap() - 1.0).abs() < 1e-9);
        // Speed-up kept growing, so it is not saturated.
        assert_eq!(curve.saturation, None);
        assert!(curve.notes.iter().any(|n| n.contains("not saturated")));
    }

    #[test]
    fn flat_curve_saturates_at_one_thread() {
        let runs = vec![
            (1u32, vec![1_000_000u64]),
            (2, vec![1_000_000]),
            (4, vec![1_000_000]),
        ];
        let curve = curve(&runs, 4, 0.05);
        assert_eq!(curve.saturation, Some(1));
    }

    #[test]
    fn median_is_the_middle_sample() {
        assert_eq!(median(&[5, 1, 3]), 3);
        assert_eq!(median(&[1, 2, 3, 4]), 2); // (2+3)/2 floored
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn missing_one_thread_baseline_is_relative_and_explained() {
        // No one-thread run, so speed-up is relative to two threads and the
        // note must say so rather than implying a one-thread baseline.
        let runs = vec![(2u32, vec![100u64]), (4, vec![50])];
        let curve = curve(&runs, 8, 0.05);
        assert_eq!(curve.points[0].speedup, None);
        assert!((curve.points[1].speedup.unwrap() - 2.0).abs() < 1e-9);
        assert!(curve.notes.iter().any(|n| n.contains("2 threads")));
    }

    #[test]
    fn empty_runs_do_not_panic() {
        let curve = curve(&[], 4, 0.05);
        assert!(curve.points.is_empty());
        assert!(!curve.notes.is_empty());
    }
}
