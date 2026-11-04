//! Top-down microarchitecture breakdown from hardware PMU counters.
//!
//! The top-down method attributes every issue slot in a pipeline to one of four
//! buckets — retiring, bad speculation, front-end bound, back-end bound — and
//! the four fractions sum to one. It answers a question the simulated counters
//! cannot: not "how many instructions" but "where did the cycles go".
//!
//! The numbers come from `perf stat --topdown`, which reads the hardware PMU
//! through `perf_event_open`. That makes them a *measurement*, and a
//! machine-dependent one, so they are labelled `measured` but are never a
//! ranking signal, exactly like the flat hardware counters already in the
//! capsule.
//!
//! The parser here reads perf's own JSON (`perf stat -j`). perf emits one
//! object per counter and, in top-down mode, one metric per bucket with a
//! `metric-value` that is a percentage string. The report keeps the raw
//! percentage alongside the parsed fraction so a reader can check the parse
//! without rerunning anything.

use serde::{Deserialize, Serialize};

/// The four top-down buckets, as fractions of the issue slots counted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopDownBreakdown {
    /// Fraction of slots where a uop retired.
    pub retiring: f64,
    /// Fraction of slots lost to mispredicted branches and machine clears.
    pub bad_speculation: f64,
    /// Fraction of slots the front end could not supply a uop for.
    pub frontend_bound: f64,
    /// Fraction of slots the back end could not accept a uop in.
    pub backend_bound: f64,
}

impl TopDownBreakdown {
    /// Sum of the four fractions. A well-formed split sums to one.
    pub fn total(&self) -> f64 {
        self.retiring + self.bad_speculation + self.frontend_bound + self.backend_bound
    }

    /// The bucket with the largest fraction, and its share.
    pub fn dominant(&self) -> (&'static str, f64) {
        let buckets = [
            ("retiring", self.retiring),
            ("bad speculation", self.bad_speculation),
            ("front-end bound", self.frontend_bound),
            ("back-end bound", self.backend_bound),
        ];
        buckets
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(("retiring", 0.0))
    }
}

/// A complete top-down result for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopDownReport {
    /// The split, when perf produced every bucket.
    pub breakdown: Option<TopDownBreakdown>,
    /// Raw `metric-value` strings perf reported, keyed by bucket name, so the
    /// parse can be checked against the source.
    pub raw_metrics: Vec<(String, String)>,
    /// Why the split is absent, when it is.
    pub unavailable_reason: Option<String>,
    /// Honesty notes about what the numbers mean.
    pub notes: Vec<String>,
}

/// Canonical bucket for a perf top-down metric name.
///
/// perf names the buckets after the kernel's metric definitions, which are
/// stable across Intel and AMD: `tma_retiring`, `tma_bad_speculation`,
/// `tma_frontend_bound`, `tma_backend_bound`. The level-two metrics share the
/// prefix (`tma_backend_bound_aux`), so only the exact level-one names match.
fn bucket_for(metric: &str) -> Option<&'static str> {
    let lower = metric.to_ascii_lowercase();
    if lower == "tma_retiring" || lower == "retiring" {
        Some("retiring")
    } else if lower == "tma_bad_speculation" || lower == "bad_speculation" {
        Some("bad speculation")
    } else if lower == "tma_frontend_bound" || lower == "frontend_bound" {
        Some("front-end bound")
    } else if lower == "tma_backend_bound" || lower == "backend_bound" {
        Some("back-end bound")
    } else {
        None
    }
}

#[derive(Deserialize)]
struct RawEntry {
    // perf spells these keys with hyphens, so they must be renamed explicitly.
    #[serde(default, rename = "metric-value")]
    metric_value: Option<serde_json::Value>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    metric: Option<String>,
}

/// Parse the buckets out of `perf stat --topdown -j` output.
///
/// perf writes one JSON object per line. A metric line carries `metric-value`
/// (a percentage, sometimes with a trailing unit) and names its metric either
/// in `event` or `metric`. Returns the fractions and the raw strings; returns
/// `None` when the output carries no recognisable bucket, which is what a
/// denied or unsupported PMU produces.
pub fn parse_topdown_json(text: &str) -> Option<TopDownReport> {
    let mut retiring: Option<f64> = None;
    let mut bad_speculation: Option<f64> = None;
    let mut frontend: Option<f64> = None;
    let mut backend: Option<f64> = None;
    let mut raw: Vec<(String, String)> = Vec::new();
    let mut saw_any_metric = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "{" || line == "}" {
            continue;
        }
        // perf may wrap its objects in a top-level `{ "counter-value" ... }`
        // stream; tolerate a trailing comma from its line-oriented printer.
        let line = line.trim_end_matches(',');
        let Ok(entry) = serde_json::from_str::<RawEntry>(line) else {
            continue;
        };
        let name = entry
            .event
            .clone()
            .or_else(|| entry.metric.clone())
            .unwrap_or_default();
        let Some(bucket) = bucket_for(&name) else {
            continue;
        };
        saw_any_metric = true;
        let Some(value) = entry.metric_value else {
            continue;
        };
        let text_value = match &value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        // Strip a trailing unit such as `%` and any whitespace.
        let numeric = text_value.trim().trim_end_matches('%').trim();
        let Ok(parsed) = numeric.parse::<f64>() else {
            continue;
        };
        // perf reports top-down as a percentage; keep the fraction.
        let fraction = parsed / 100.0;
        raw.push((bucket.to_string(), text_value.clone()));
        match bucket {
            "retiring" => retiring = Some(fraction),
            "bad speculation" => bad_speculation = Some(fraction),
            "front-end bound" => frontend = Some(fraction),
            "back-end bound" => backend = Some(fraction),
            _ => {}
        }
    }

    let mut notes = vec![
        "measured from hardware PMU counters via perf_event_open; machine-dependent, \
         never a ranking signal"
            .to_string(),
    ];

    if !saw_any_metric {
        return Some(TopDownReport {
            breakdown: None,
            raw_metrics: raw,
            unavailable_reason: Some(
                "perf reported no top-down buckets; this PMU or kernel does not expose \
                 the required events"
                    .to_string(),
            ),
            notes,
        });
    }

    let present = [retiring, bad_speculation, frontend, backend];
    if present.iter().any(|value| value.is_none()) {
        let missing: Vec<&str> = [
            "retiring",
            "bad speculation",
            "front-end bound",
            "back-end bound",
        ]
        .into_iter()
        .zip(present)
        .filter_map(|(name, value)| value.is_none().then_some(name))
        .collect();
        notes.push(format!(
            "perf did not report every bucket (missing: {}); the split is not computed \
             rather than filled with zeros",
            missing.join(", ")
        ));
        return Some(TopDownReport {
            breakdown: None,
            raw_metrics: raw,
            unavailable_reason: Some(format!(
                "this PMU did not report all four top-down buckets (missing: {})",
                missing.join(", ")
            )),
            notes,
        });
    }

    let breakdown = TopDownBreakdown {
        retiring: retiring.unwrap_or(0.0),
        bad_speculation: bad_speculation.unwrap_or(0.0),
        frontend_bound: frontend.unwrap_or(0.0),
        backend_bound: backend.unwrap_or(0.0),
    };
    let total = breakdown.total();
    if (total - 1.0).abs() > 0.02 {
        notes.push(format!(
            "the four buckets sum to {total:.3}, not 1.0; the PMU may have multiplexed \
             the counters, so treat the split as approximate"
        ));
    }
    let (bucket, share) = breakdown.dominant();
    notes.push(format!(
        "the largest share of issue slots went to `{bucket}` ({:.0}%)",
        share * 100.0
    ));

    Some(TopDownReport {
        breakdown: Some(breakdown),
        raw_metrics: raw,
        unavailable_reason: None,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real shape of `perf stat --topdown -j` output: one JSON object per line,
    /// a metric line carrying `metric-value` as a percentage string. The
    /// counter lines around the metrics are here because they are what the
    /// parser must skip.
    const PERF_TOP_DOWN: &str = r#"{"counter-value" : "1000000", "unit" : "", "event" : "cycles", "event-runtime" : 1000000, "pcnt-running" : 100.00, "metric-value" : "1.000000", "metric-unit" : "insn per cycle"}
{"counter-value" : "0.600000", "unit" : "", "event" : "tma_retiring", "event-runtime" : 0, "pcnt-running" : 0.00, "metric-value" : "60.00", "metric-unit" : "%"}
{"counter-value" : "0.100000", "unit" : "", "event" : "tma_bad_speculation", "event-runtime" : 0, "pcnt-running" : 0.00, "metric-value" : "10.00", "metric-unit" : "%"}
{"counter-value" : "0.050000", "unit" : "", "event" : "tma_frontend_bound", "event-runtime" : 0, "pcnt-running" : 0.00, "metric-value" : "5.00", "metric-unit" : "%"}
{"counter-value" : "0.250000", "unit" : "", "event" : "tma_backend_bound", "event-runtime" : 0, "pcnt-running" : 0.00, "metric-value" : "25.00", "metric-unit" : "%"}"#;

    #[test]
    fn a_complete_topdown_split_is_parsed_and_sums_to_one() {
        let report = parse_topdown_json(PERF_TOP_DOWN).expect("output parses");
        let breakdown = report.breakdown.expect("split is present");
        assert!((breakdown.retiring - 0.6).abs() < 1e-9);
        assert!((breakdown.bad_speculation - 0.1).abs() < 1e-9);
        assert!((breakdown.frontend_bound - 0.05).abs() < 1e-9);
        assert!((breakdown.backend_bound - 0.25).abs() < 1e-9);
        assert!((breakdown.total() - 1.0).abs() < 1e-9);
        assert!(report.unavailable_reason.is_none());
    }

    #[test]
    fn raw_metric_strings_are_kept_for_audit() {
        let report = parse_topdown_json(PERF_TOP_DOWN).expect("output parses");
        assert!(report
            .raw_metrics
            .iter()
            .any(|(bucket, value)| bucket == "retiring" && value == "60.00"));
    }

    #[test]
    fn the_dominant_bucket_is_reported() {
        let report = parse_topdown_json(PERF_TOP_DOWN).unwrap();
        let breakdown = report.breakdown.unwrap();
        assert_eq!(breakdown.dominant().0, "retiring");
    }

    #[test]
    fn output_with_no_buckets_is_unavailable_not_zero() {
        let denied = "Error:\nNo permission to enable tma_retiring event.\n";
        let report = parse_topdown_json(denied).expect("even a denial is a report");
        assert!(report.breakdown.is_none());
        assert!(report
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("no top-down buckets"));
    }

    #[test]
    fn a_missing_bucket_yields_no_split_not_zeros() {
        let partial = r#"{"event" : "tma_retiring", "metric-value" : "60.00"}
{"event" : "tma_bad_speculation", "metric-value" : "10.00"}
{"event" : "tma_frontend_bound", "metric-value" : "5.00"}"#;
        let report = parse_topdown_json(partial).expect("output parses");
        assert!(report.breakdown.is_none());
        assert!(report
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("back-end bound"));
    }

    #[test]
    fn a_split_that_does_not_sum_to_one_is_flagged() {
        let skewed = r#"{"event" : "tma_retiring", "metric-value" : "40.00"}
{"event" : "tma_bad_speculation", "metric-value" : "10.00"}
{"event" : "tma_frontend_bound", "metric-value" : "10.00"}
{"event" : "tma_backend_bound", "metric-value" : "10.00"}"#;
        let report = parse_topdown_json(skewed).expect("output parses");
        assert!(report.breakdown.is_some());
        assert!(report.notes.iter().any(|note| note.contains("sum to")));
    }

    #[test]
    fn level_two_metrics_do_not_match_the_level_one_buckets() {
        // The level-two metrics share the level-one prefix but must not be
        // mistaken for the four buckets.
        let with_level_two = format!(
            "{PERF_TOP_DOWN}\n{{\"event\" : \"tma_backend_bound_aux\", \"metric-value\" : \"30.00\"}}"
        );
        let report = parse_topdown_json(&with_level_two).expect("output parses");
        assert!(report.breakdown.is_some());
        assert_eq!(report.raw_metrics.len(), 4);
        assert!(!report.raw_metrics.iter().any(|(_, value)| value == "30.00"));
    }
}
