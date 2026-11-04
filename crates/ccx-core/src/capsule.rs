//! Experiment capsules: the unit of evidence.
//!
//! A capsule is a versioned JSON document describing one measured run in
//! enough detail to reproduce it. See `CAPSULE_SPEC.md` for the contract this
//! module implements.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub mod migrate;
pub mod repeatability;

/// The schema version this build writes.
pub const CURRENT_SCHEMA_VERSION: u32 = 19;
pub const LEGACY_BINARY_DIGEST_UNAVAILABLE: &str = "unavailable_legacy";
pub const DEFAULT_BUILD_REPEAT: u32 = 3;

/// Measurement harness version, bumped when measurement behaviour changes.
pub const HARNESS_VERSION: &str = "0.19.0";
/// Stable identifier of the fixed host-noise canary workload.
pub const NOISE_CANARY_WORKLOAD: &str = "ccx-canary-v1";
/// Outlier flag: a sample is *slow* when it exceeds the median by this factor.
pub const TIMING_SLOW_OUTLIER_FACTOR: f64 = 2.0;
/// Outlier flag: a sample is *fast* when it falls below the median by this
/// factor. A too-fast run usually means the work was skipped or the clock was
/// perturbed, so it is flagged rather than trusted.
pub const TIMING_FAST_OUTLIER_FACTOR: f64 = 0.5;

/// Hostname forced on every measured container.
///
/// Docker assigns a random 12-hex hostname to each `docker run`, and it
/// exports that value to the container as the `HOSTNAME` environment
/// variable. glibc's `__tunables_init` walks the environment at startup, so
/// its cost depends on the variable count and lengths; a random hostname
/// therefore shifts the loader's instruction and branch counts by a few
/// counts between runs. Pinning the hostname makes the measured environment
/// byte-identical everywhere. The value matches Docker's default length (12
/// characters) so already-recorded capsules keep the same startup cost.
pub const CCX_HOSTNAME: &str = "ccx-measure1";

/// Canonical environment for both compilation and execution. Every other
/// variable is dropped so counts cannot drift between runs.
pub const FIXED_ENV: &[(&str, &str)] = &[
    (
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ),
    ("LANG", "C"),
    ("LC_ALL", "C"),
    ("TZ", "UTC"),
    ("SOURCE_DATE_EPOCH", "0"),
];

pub fn utc_timestamp_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format_utc_timestamp(seconds)
}

fn format_utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[derive(Debug, thiserror::Error)]
pub enum CapsuleError {
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported schema_version {found}; this build understands up to {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("capsule failed validation: {0}")]
    Invalid(String),
    #[error("capsule_id mismatch: document says {claimed}, content hashes to {computed}")]
    IdMismatch { claimed: String, computed: String },
}

/// How a number was obtained. This label is mandatory on every metric and is
/// what keeps the platform honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// A real hardware or OS counter.
    Measured,
    /// A deterministic model (e.g. cachegrind). Identical on any machine.
    Simulated,
    /// Wall-clock. Machine-dependent, never used for ranking.
    Indicative,
}

impl fmt::Display for MetricKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MetricKind::Measured => "measured",
            MetricKind::Simulated => "simulated",
            MetricKind::Indicative => "indicative",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricMethod {
    Cachegrind,
    Perf,
    Getrusage,
    /// The kernel's cgroup memory high-water mark (`memory.peak` on cgroup v2,
    /// `memory.max_usage_in_bytes` on v1). A measurement of the memory charged
    /// to the run's cgroup, which is what a container memory limit governs.
    Cgroup,
    Stat,
    Manual,
    ValgrindMemcheck,
    WallClock,
    Runner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyKind {
    Stdev,
    Ci95,
    Range,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uncertainty {
    pub kind: UncertaintyKind,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub kind: MetricKind,
    pub method: MetricMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<Uncertainty>,
}

impl Metric {
    pub fn count(name: &str, value: u64, kind: MetricKind, method: MetricMethod) -> Self {
        Metric {
            name: name.to_string(),
            value: value as f64,
            unit: "count".to_string(),
            kind,
            method,
            uncertainty: None,
        }
    }

    pub fn bytes(name: &str, value: u64, kind: MetricKind, method: MetricMethod) -> Self {
        Metric {
            name: name.to_string(),
            value: value as f64,
            unit: "bytes".to_string(),
            kind,
            method,
            uncertainty: None,
        }
    }

    pub fn machine_dependent(&self) -> bool {
        self.kind == MetricKind::Indicative
            || matches!(
                self.method,
                MetricMethod::Getrusage
                    | MetricMethod::Cgroup
                    | MetricMethod::Perf
                    | MetricMethod::WallClock
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    C,
    Cpp,
    Rust,
    D,
    Zig,
    Odin,
    Go,
    #[serde(rename = "asm-x86-64")]
    AsmX86_64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainName {
    Gcc,
    #[serde(rename = "gcc-13")]
    Gcc13,
    Clang,
    #[serde(rename = "clang-18")]
    Clang18,
    Rust,
    Gdc,
    Zig,
    Odin,
    Go,
}

impl ToolchainName {
    pub fn profile_id(self) -> &'static str {
        match self {
            Self::Gcc => "gcc",
            Self::Gcc13 => "gcc-13",
            Self::Clang => "clang",
            Self::Clang18 => "clang-18",
            Self::Rust => "rust",
            Self::Gdc => "gdc",
            Self::Zig => "zig",
            Self::Odin => "odin",
            Self::Go => "go",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedSet {
    Public,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProblemRef {
    pub id: String,
    pub statement_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub language: Language,
    pub entry: String,
    /// relative path -> "sha256:<hex>"
    pub files: BTreeMap<String, String>,
    pub source_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub name: ToolchainName,
    pub version: String,
    pub image: String,
    pub image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_fingerprint: Option<String>,
    pub libc: String,
}

impl Toolchain {
    pub fn same_environment(&self, other: &Self) -> bool {
        match (
            self.environment_fingerprint.as_deref(),
            other.environment_fingerprint.as_deref(),
        ) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.image_digest == other.image_digest,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub argv: Vec<String>,
    pub opt_level: String,
    pub binary_sha256: String,
    pub build_repeat: u32,
    pub env: BTreeMap<String, String>,
    pub argv_program: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv_runner: Vec<String>,
    pub stdin_sha256: Option<String>,
    pub stack_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceBuild {
    pub binary_sha256: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    pub version: String,
    pub metric_method: MetricMethod,
    pub ruleset: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    pub generator: String,
    pub seed: u64,
    pub seed_set: SeedSet,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub cpu_model: String,
    pub arch: String,
    pub os: String,
    pub kernel: String,
    pub nproc: u32,
    pub frequency_policy: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accepted,
    WrongAnswer,
    TimeLimit,
    MemoryLimit,
    RuntimeError,
    BuildError,
    RejectedUnsafe,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Verdict::Accepted => "accepted",
            Verdict::WrongAnswer => "wrong answer",
            Verdict::TimeLimit => "time limit",
            Verdict::MemoryLimit => "memory limit",
            Verdict::RuntimeError => "runtime error",
            Verdict::BuildError => "build error",
            Verdict::RejectedUnsafe => "rejected unsafe",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeatability {
    pub runs: u32,
    pub per_metric: BTreeMap<String, Spread>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spread {
    #[serde(default)]
    pub samples: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<f64>>,
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub spread_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p99: Option<f64>,
}

impl Spread {
    fn matches_values(&self) -> bool {
        let Some(values) = &self.values else {
            return true;
        };
        if values.len() != self.samples as usize || values.iter().any(|value| !value.is_finite()) {
            return false;
        }
        let Some(summary) = repeatability::spread(values) else {
            return false;
        };
        self.min == summary.min
            && self.max == summary.max
            && self.median == summary.median
            && self.spread_pct == summary.spread_pct
            && self.p95 == summary.p95
            && self.p99 == summary.p99
    }
}

/// The deterministic metric names a cachegrind-simulated run reports. The
/// tolerance policies below key off this set so a policy can never silently
/// cover a metric that is not actually simulated.
pub const SIMULATED_METRICS: &[&str] = &[
    "instructions",
    "cache_misses",
    "branch_misses",
    "instruction_cache_misses",
    "data_cache_misses",
    "data_refs",
    "data_reads",
    "data_writes",
    "branch_instructions",
    "cache_miss_rate",
    "branch_miss_rate",
    "memory_refs",
    "branches",
];

/// A declared tolerance for a *simulated* metric that has a known,
/// machine-dependent startup cost.
///
/// Simulated counts are normally required to be identical across machines.
/// Some runtimes, however, execute machine-dependent work before `main` that
/// no container setting can neutralize. A policy names that cost, bounds it,
/// and lists the exact metrics it covers, so the cross-machine gate can report
/// a documented, understood spread instead of hiding it. A policy never widens
/// the within-host repeatability gate and never applies to a measured metric.
#[derive(Debug, Clone, PartialEq)]
pub struct TolerancePolicy {
    /// Stable identifier recorded in the capsule.
    pub id: &'static str,
    /// Why the machine-dependent cost exists.
    pub reason: &'static str,
    /// The toolchain the policy applies to, as `ToolchainName::profile_id`.
    pub toolchain: &'static str,
    /// Maximum relative cross-machine spread the policy permits, e.g. `1e-6`.
    pub relative_tolerance: f64,
    /// The exact metric names the policy covers.
    pub metrics: &'static [&'static str],
}

/// Declared simulated-metric tolerances, one entry per known machine-dependent
/// startup cost. Empty by default: every language and metric stays exact.
///
/// Both bounds are set just above the largest cross-host spread the hosted
/// runners have actually shown (0.043% for the C++/Rust `branch_misses` count
/// and 0.007% for D), so they admit a documented per-host offset without
/// swallowing a real algorithmic change: every metric is still a count of
/// simulated events, and the gate continues to require each host's 20 raw
/// samples to be byte-identical.
pub const TOLERANCE_POLICIES: &[TolerancePolicy] = &[
    TolerancePolicy {
        id: "d-gdc-druntime-cpuid",
        reason: "D druntime runs core.cpuid at startup; its cost depends on the CPU \
                 model and cannot be normalized from inside the container",
        toolchain: "gdc",
        relative_tolerance: 1e-3,
        metrics: SIMULATED_METRICS,
    },
    TolerancePolicy {
        id: "rust-libc-ifunc-dispatch",
        reason: "the Rust runtime and libc resolve CPU-feature (ifunc) variants before \
                 main, so the host CPU model adds a small fixed offset to the simulated \
                 counts that no container setting can neutralize",
        toolchain: "rust",
        relative_tolerance: 1e-3,
        metrics: SIMULATED_METRICS,
    },
];

/// The tolerance policy that covers `metric_name` for `toolchain_profile`, if
/// any. Matching requires both the toolchain and the metric name to line up.
pub fn tolerance_policy_for(
    toolchain_profile: &str,
    metric_name: &str,
) -> Option<&'static TolerancePolicy> {
    TOLERANCE_POLICIES.iter().find(|policy| {
        policy.toolchain == toolchain_profile && policy.metrics.contains(&metric_name)
    })
}

/// A tolerance policy that applied to a recorded run, with the spread the run
/// itself observed. Recorded in the capsule so no tolerance is ever silent.
/// The spread here is this run's own (within-host) spread; the cross-machine
/// spread is checked by the comparison tool, not observed by a single capsule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedTolerance {
    /// Matches `TolerancePolicy::id`.
    pub policy: String,
    /// The toolchain the policy was applied for.
    pub toolchain: String,
    /// Maximum permitted relative spread.
    pub relative_tolerance: f64,
    /// The metric names the policy covered in this capsule.
    pub metrics: Vec<String>,
    /// The largest relative spread this run observed across those metrics.
    pub observed_relative_spread: f64,
}

impl AppliedTolerance {
    /// Build the record for a toolchain, or `None` when no policy applies.
    /// `spreads` pairs each metric name with its relative spread in this run.
    pub fn record(toolchain_profile: &str, spreads: &[(String, f64)]) -> Option<Self> {
        let policy = TOLERANCE_POLICIES
            .iter()
            .find(|policy| policy.toolchain == toolchain_profile)?;
        let mut metrics = Vec::new();
        let mut observed: f64 = 0.0;
        for (name, spread) in spreads {
            if policy.metrics.iter().any(|metric| metric == name) {
                metrics.push(name.clone());
                observed = observed.max(spread.abs());
            }
        }
        Some(AppliedTolerance {
            policy: policy.id.to_string(),
            toolchain: policy.toolchain.to_string(),
            relative_tolerance: policy.relative_tolerance,
            metrics,
            observed_relative_spread: observed,
        })
    }
}

/// Host-noise canary recorded alongside a measurement.
///
/// A fixed reference workload is measured immediately before and after the
/// solution's own measurement. The workload and its input are identical both
/// times, so any change in its wall clock is host noise — a busy neighbour, a
/// frequency-policy change, or a machine that throttled mid-run — and not a
/// property of the solution. The status is advisory: it never changes the
/// solution's deterministic metrics and never fails a measurement, it only
/// tells a reader whether to trust the *indicative* numbers in this capsule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseCanary {
    /// Stable identifier of the fixed workload, e.g. `ccx-canary-v1`.
    pub workload: String,
    /// The canary's own wall clock before the solution was measured, in ns.
    pub before_wall_ns: f64,
    /// The canary's own wall clock after the solution was measured, in ns.
    pub after_wall_ns: f64,
    /// Relative drift between the two canary runs, as a percentage.
    pub drift_pct: f64,
    /// The drift tolerance the status was judged against, as a percentage.
    pub tolerance_pct: f64,
    /// `stable`, `drifted`, or `unavailable` when the canary could not run.
    pub status: String,
    /// Why the canary is `unavailable`, when it could not run. Absent when the
    /// canary ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl NoiseCanary {
    /// Default drift tolerance. Deliberately generous: the canary is meant to
    /// catch gross drift (a runaway neighbour, a machine that throttled), not
    /// ordinary run-to-run jitter, and a false flag would train readers to
    /// ignore it.
    pub const DEFAULT_TOLERANCE_PCT: f64 = 20.0;

    /// Build a canary record from the two wall-clock samples. `before` and
    /// `after` are the canary's own wall clocks in nanoseconds.
    pub fn from_samples(workload: &str, before_wall_ns: f64, after_wall_ns: f64) -> Self {
        let drift_pct = relative_drift_pct(before_wall_ns, after_wall_ns);
        let status = if drift_pct > Self::DEFAULT_TOLERANCE_PCT {
            "drifted"
        } else {
            "stable"
        };
        NoiseCanary {
            workload: workload.to_string(),
            before_wall_ns,
            after_wall_ns,
            drift_pct,
            tolerance_pct: Self::DEFAULT_TOLERANCE_PCT,
            status: status.to_string(),
            note: None,
        }
    }

    /// Record that the canary could not be run, with the reason kept honest.
    pub fn unavailable(workload: &str, reason: impl Into<String>) -> Self {
        NoiseCanary {
            workload: workload.to_string(),
            before_wall_ns: 0.0,
            after_wall_ns: 0.0,
            drift_pct: 0.0,
            tolerance_pct: Self::DEFAULT_TOLERANCE_PCT,
            status: "unavailable".to_string(),
            note: Some(reason.into()),
        }
    }

    /// True when the canary ran and found the host stable.
    pub fn is_stable(&self) -> bool {
        self.status == "stable"
    }
}

/// Relative drift between two positive samples, as a percentage of their
/// smaller value, so the result is symmetric in `before`/`after`.
pub fn relative_drift_pct(before: f64, after: f64) -> f64 {
    let smaller = before.abs().min(after.abs());
    if smaller <= 0.0 {
        return 0.0;
    }
    (before - after).abs() / smaller * 100.0
}

/// One wall-clock observation, kept with its run index so the reader can see
/// *which* run was slow rather than only that one was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingObservation {
    /// Zero-based index of this run within the measured sequence.
    pub run: u32,
    /// Wall clock for this run, in nanoseconds.
    pub wall_ns: f64,
    /// `true` when this sample was flagged as an outlier against the median.
    pub outlier: bool,
    /// `slow` or `fast` when `outlier` is set; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

/// Warm-up evidence, derived from the leading wall-clock samples.
///
/// The first runs of a fresh container are usually slower while caches and CPU
/// frequency settle. Rather than run extra unrecorded samples, the diagnostic
/// looks at the samples it already has: how many leading runs were slow
/// outliers, and how much the very first run cost above the median.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarmUp {
    /// Number of leading runs, from the first, that were flagged as slow
    /// outliers. A nonzero value is evidence the host had not settled when
    /// sampling began.
    pub leading_slow_runs: u32,
    /// The first run's wall clock as a percentage above the median; zero when
    /// the first run was at or below the median.
    pub first_run_penalty_pct: f64,
}

/// Wall-clock diagnostics: outlier flags and warm-up evidence.
///
/// These describe the *indicative* wall-clock samples only. They never change a
/// deterministic metric, a rank, or a verdict; they exist so a reader can tell a
/// settled host from a noisy one and see which runs were discarded as noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingDiagnostic {
    /// Every candidate wall-clock sample, with its run index and outlier flag.
    pub samples: Vec<TimingObservation>,
    /// Run indices flagged as outliers, in order.
    pub outlier_runs: Vec<u32>,
    /// Median of the samples *with outliers removed*; `None` when removing
    /// every flagged sample would leave too few to summarize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trimmed_median_ns: Option<f64>,
    /// The factor above the median at which a sample is flagged *slow*.
    pub slow_factor: f64,
    /// The factor below the median at which a sample is flagged *fast*.
    pub fast_factor: f64,
    /// Warm-up evidence derived from the leading samples.
    pub warm_up: WarmUp,
}

impl TimingDiagnostic {
    /// Classify wall-clock samples: flag slow/fast outliers against the median,
    /// report a trimmed median, and summarize warm-up evidence.
    ///
    /// The classification is deterministic and order-independent, so replaying
    /// the same samples reproduces the same flags.
    pub fn analyse(samples: &[u64]) -> Self {
        let values: Vec<f64> = samples.iter().map(|v| *v as f64).collect();
        let median = repeatability::median(&values).unwrap_or(0.0);
        let slow_at = median * TIMING_SLOW_OUTLIER_FACTOR;
        let fast_at = median * TIMING_FAST_OUTLIER_FACTOR;
        let observations: Vec<TimingObservation> = values
            .iter()
            .enumerate()
            .map(|(run, wall_ns)| {
                let direction = if median > 0.0 && *wall_ns > slow_at {
                    Some("slow")
                } else if median > 0.0 && *wall_ns < fast_at {
                    Some("fast")
                } else {
                    None
                };
                TimingObservation {
                    run: run as u32,
                    wall_ns: *wall_ns,
                    outlier: direction.is_some(),
                    direction: direction.map(str::to_string),
                }
            })
            .collect();
        let outlier_runs: Vec<u32> = observations
            .iter()
            .filter(|observation| observation.outlier)
            .map(|observation| observation.run)
            .collect();
        // Only trim when enough clean samples remain to be a median rather than
        // a single value in disguise.
        let kept: Vec<f64> = observations
            .iter()
            .filter(|observation| !observation.outlier)
            .map(|observation| observation.wall_ns)
            .collect();
        let trimmed_median_ns = (kept.len() >= 3)
            .then(|| repeatability::median(&kept))
            .flatten();
        let leading_slow_runs = observations
            .iter()
            .take_while(|observation| observation.direction.as_deref() == Some("slow"))
            .count() as u32;
        let first_run_penalty_pct = if median > 0.0 {
            let first = observations.first().map(|o| o.wall_ns).unwrap_or(0.0);
            ((first - median) / median * 100.0).max(0.0)
        } else {
            0.0
        };
        TimingDiagnostic {
            samples: observations,
            outlier_runs,
            trimmed_median_ns,
            slow_factor: TIMING_SLOW_OUTLIER_FACTOR,
            fast_factor: TIMING_FAST_OUTLIER_FACTOR,
            warm_up: WarmUp {
                leading_slow_runs,
                first_run_penalty_pct,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capsule {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule_id: Option<String>,
    pub created_at: String,
    pub problem: ProblemRef,
    pub source: SourceRef,
    pub toolchain: Toolchain,
    pub build: Build,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_build: Option<ReferenceBuild>,
    pub harness: Harness,
    pub input: InputSpec,
    pub host: Host,
    pub verdict: Verdict,
    pub metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_tolerance: Option<AppliedTolerance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_canary: Option<NoiseCanary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<TimingDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeatability: Option<Repeatability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_repeatability: Option<Repeatability>,
}

impl Capsule {
    fn same_build_recipe(&self, other: &Self) -> bool {
        let before: Vec<&str> = self
            .build
            .argv
            .iter()
            .map(String::as_str)
            .filter(|argument| !argument.starts_with("/src0/"))
            .collect();
        let after: Vec<&str> = other
            .build
            .argv
            .iter()
            .map(String::as_str)
            .filter(|argument| !argument.starts_with("/src0/"))
            .collect();
        let before_configuration: Vec<_> = self
            .source
            .files
            .iter()
            .filter(|(path, _)| {
                path.as_str() == "ccx.toml"
                    || path.as_str() == "CMakeLists.txt"
                    || path.ends_with(".cmake")
            })
            .collect();
        let after_configuration: Vec<_> = other
            .source
            .files
            .iter()
            .filter(|(path, _)| {
                path.as_str() == "ccx.toml"
                    || path.as_str() == "CMakeLists.txt"
                    || path.ends_with(".cmake")
            })
            .collect();
        self.build.opt_level == other.build.opt_level
            && before == after
            && before_configuration == after_configuration
    }

    pub fn same_problem_definition(&self, other: &Self) -> bool {
        self.problem.id == other.problem.id
            && self.problem.statement_sha256 == other.problem.statement_sha256
            && self
                .source
                .files
                .iter()
                .filter(|(path, _)| path.starts_with("problem/"))
                .eq(other
                    .source
                    .files
                    .iter()
                    .filter(|(path, _)| path.starts_with("problem/")))
    }

    pub fn same_comparison_context(&self, other: &Self) -> bool {
        self.same_problem_definition(other)
            && self.source.language == other.source.language
            && self.toolchain.name == other.toolchain.name
            && self.toolchain.version == other.toolchain.version
            && self.toolchain.same_environment(&other.toolchain)
            && self.same_build_recipe(other)
            && self.harness.version == other.harness.version
            && self.build.stdin_sha256.is_some()
            && self.build.stdin_sha256 == other.build.stdin_sha256
            && self.build.argv_program == other.build.argv_program
            && self.build.argv_runner == other.build.argv_runner
            && self.build.env == other.build.env
            && self.build.stack_bytes == other.build.stack_bytes
            && self.host.arch == other.host.arch
            && self.host.os == other.host.os
    }

    /// Canonical JSON of the body with `capsule_id` removed and keys sorted.
    /// `serde_json::Value` maps are `BTreeMap`-backed, so serialisation is
    /// already key-sorted, which makes this stable across builds.
    pub fn canonical_body(&self) -> Result<String, CapsuleError> {
        let mut value = serde_json::to_value(self)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.remove("capsule_id");
        }
        Ok(serde_json::to_string(&value)?)
    }

    pub fn compute_id(&self) -> Result<String, CapsuleError> {
        Ok(sha256_hex(self.canonical_body()?.as_bytes()))
    }

    /// Fill in `capsule_id` from the current content.
    pub fn seal(&mut self) -> Result<(), CapsuleError> {
        let id = self.compute_id()?;
        self.capsule_id = Some(id);
        Ok(())
    }

    /// Full validation: schema version, structural rules, and identity.
    pub fn validate(&self) -> Result<(), CapsuleError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(CapsuleError::UnsupportedVersion {
                found: self.schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        if self.source.files.is_empty() {
            return Err(CapsuleError::Invalid("source.files is empty".into()));
        }
        if !self.source.files.contains_key(&self.source.entry) {
            return Err(CapsuleError::Invalid(format!(
                "source.entry {:?} is not present in source.files",
                self.source.entry
            )));
        }
        for (path, hash) in &self.source.files {
            if path.is_empty()
                || std::path::Path::new(path)
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(CapsuleError::Invalid(format!(
                    "source file path {path:?} must be a normalized relative path"
                )));
            }
            let digest = hash.strip_prefix("sha256:").unwrap_or("");
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(CapsuleError::Invalid(format!(
                    "source.files[{path:?}] must be \"sha256:<64 hex>\""
                )));
            }
        }

        let expected_source = hash_source_files(&self.source.files);
        if expected_source != self.source.source_sha256 {
            return Err(CapsuleError::Invalid(format!(
                "source.source_sha256 mismatch: document says {}, recomputed {}",
                self.source.source_sha256, expected_source
            )));
        }

        // A digest is a contract; a tag is not.
        let image_digest = self
            .toolchain
            .image_digest
            .strip_prefix("sha256:")
            .unwrap_or("");
        if image_digest.len() != 64 || !image_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CapsuleError::Invalid(
                "toolchain.image_digest must be \"sha256:<hex>\"; tags like :latest are not allowed"
                    .into(),
            ));
        }
        if self.toolchain.image.contains(":latest") {
            return Err(CapsuleError::Invalid(
                "toolchain.image must be pinned by digest, not :latest".into(),
            ));
        }
        if self.build.binary_sha256 != LEGACY_BINARY_DIGEST_UNAVAILABLE {
            let binary_sha256 = &self.build.binary_sha256;
            let digest = binary_sha256.strip_prefix("sha256:").unwrap_or("");
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(CapsuleError::Invalid(
                    "build.binary_sha256 must be \"sha256:<64 hex>\"".into(),
                ));
            }
        }
        if self.build.build_repeat == 0 || self.build.build_repeat > 20 {
            return Err(CapsuleError::Invalid(
                "build.build_repeat must be between 1 and 20".into(),
            ));
        }
        if let Some(fingerprint) = &self.toolchain.environment_fingerprint {
            let digest = fingerprint.strip_prefix("sha256:").unwrap_or("");
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(CapsuleError::Invalid(
                    "toolchain.environment_fingerprint must be \"sha256:<64 hex>\"".into(),
                ));
            }
        }

        let expected_host = host_fingerprint(
            &self.host.cpu_model,
            &self.host.arch,
            &self.host.os,
            &self.host.kernel,
            self.host.nproc,
        );
        if expected_host != self.host.fingerprint {
            return Err(CapsuleError::Invalid(format!(
                "host.fingerprint mismatch: document says {}, recomputed {}",
                self.host.fingerprint, expected_host
            )));
        }

        for m in &self.metrics {
            if m.name.trim().is_empty() {
                return Err(CapsuleError::Invalid("metric with empty name".into()));
            }
            if !m.value.is_finite() {
                return Err(CapsuleError::Invalid(format!(
                    "metric {:?} has non-finite value",
                    m.name
                )));
            }
            if m.kind == MetricKind::Indicative && m.uncertainty.is_none() {
                // Indicative numbers without a spread are exactly the "fake
                // precision" the platform promises not to ship.
                return Err(CapsuleError::Invalid(format!(
                    "indicative metric {:?} must carry uncertainty",
                    m.name
                )));
            }
            if let Some(uncertainty) = &m.uncertainty {
                if !uncertainty.value.is_finite() || uncertainty.value < 0.0 {
                    return Err(CapsuleError::Invalid(format!(
                        "metric {:?} has invalid uncertainty",
                        m.name
                    )));
                }
            }
        }

        if let Some(applied) = &self.applied_tolerance {
            if !applied.relative_tolerance.is_finite()
                || applied.relative_tolerance < 0.0
                || !applied.observed_relative_spread.is_finite()
                || applied.observed_relative_spread < 0.0
            {
                return Err(CapsuleError::Invalid(
                    "applied_tolerance has a non-finite or negative spread".into(),
                ));
            }
            // A declared tolerance must name a real policy and stay within it,
            // so a widened bound can never be smuggled in through the capsule.
            let policy = TOLERANCE_POLICIES
                .iter()
                .find(|policy| policy.id == applied.policy)
                .ok_or_else(|| {
                    CapsuleError::Invalid(format!(
                        "applied_tolerance names unknown policy {:?}",
                        applied.policy
                    ))
                })?;
            if applied.toolchain != policy.toolchain {
                return Err(CapsuleError::Invalid(format!(
                    "applied_tolerance toolchain {:?} does not match policy {:?}",
                    applied.toolchain, policy.toolchain
                )));
            }
            if applied.relative_tolerance > policy.relative_tolerance {
                return Err(CapsuleError::Invalid(format!(
                    "applied_tolerance tolerance {} exceeds policy bound {}",
                    applied.relative_tolerance, policy.relative_tolerance
                )));
            }
            for name in &applied.metrics {
                if !policy.metrics.iter().any(|metric| metric == name) {
                    return Err(CapsuleError::Invalid(format!(
                        "applied_tolerance metric {name:?} is not covered by policy {:?}",
                        policy.id
                    )));
                }
                let metric = self
                    .metrics
                    .iter()
                    .find(|metric| &metric.name == name)
                    .ok_or_else(|| {
                        CapsuleError::Invalid(format!(
                            "applied_tolerance covers missing metric {name:?}"
                        ))
                    })?;
                // A policy exists only for machine-dependent *simulated*
                // startup cost; it must never excuse a measured metric.
                if metric.kind != MetricKind::Simulated {
                    return Err(CapsuleError::Invalid(format!(
                        "applied_tolerance covers non-simulated metric {name:?}"
                    )));
                }
            }
        }

        if let Some(canary) = &self.noise_canary {
            let known_status =
                matches!(canary.status.as_str(), "stable" | "drifted" | "unavailable");
            let finite = canary.before_wall_ns.is_finite()
                && canary.after_wall_ns.is_finite()
                && canary.drift_pct.is_finite()
                && canary.tolerance_pct.is_finite();
            let non_negative = canary.before_wall_ns >= 0.0
                && canary.after_wall_ns >= 0.0
                && canary.drift_pct >= 0.0
                && canary.tolerance_pct >= 0.0;
            if canary.workload.trim().is_empty() || !known_status || !finite || !non_negative {
                return Err(CapsuleError::Invalid(
                    "noise_canary has an unknown status or invalid samples".into(),
                ));
            }
            if canary.status == "unavailable" && canary.note.is_none() {
                return Err(CapsuleError::Invalid(
                    "an unavailable noise_canary must record why".into(),
                ));
            }
            // A recorded canary must be self-consistent: its drift is exactly
            // the drift of its own two samples, and its status follows from
            // that drift. Otherwise the status could disagree with the numbers.
            let expected_drift = relative_drift_pct(canary.before_wall_ns, canary.after_wall_ns);
            if (canary.drift_pct - expected_drift).abs() > 1e-9 {
                return Err(CapsuleError::Invalid(
                    "noise_canary drift does not match its own samples".into(),
                ));
            }
            if canary.status != "unavailable" {
                let expected_status = if canary.drift_pct > canary.tolerance_pct {
                    "drifted"
                } else {
                    "stable"
                };
                if canary.status != expected_status {
                    return Err(CapsuleError::Invalid(format!(
                        "noise_canary status {:?} disagrees with its drift {:.4}%",
                        canary.status, canary.drift_pct
                    )));
                }
            }
        }

        if let Some(timing) = &self.timing {
            if timing.samples.is_empty() {
                return Err(CapsuleError::Invalid(
                    "timing diagnostics must record at least one sample".into(),
                ));
            }
            if !timing.slow_factor.is_finite()
                || !timing.fast_factor.is_finite()
                || timing.slow_factor <= 0.0
                || timing.fast_factor <= 0.0
                || timing.fast_factor >= timing.slow_factor
            {
                return Err(CapsuleError::Invalid(
                    "timing diagnostics have invalid outlier factors".into(),
                ));
            }
            // The samples are a run-indexed sequence, so the run numbers must be
            // exactly 0..n with no gaps and no repeats.
            for (index, observation) in timing.samples.iter().enumerate() {
                let direction_ok = match observation.direction.as_deref() {
                    None => !observation.outlier,
                    Some("slow") | Some("fast") => observation.outlier,
                    Some(_) => false,
                };
                if observation.run != index as u32
                    || !observation.wall_ns.is_finite()
                    || observation.wall_ns < 0.0
                    || !direction_ok
                {
                    return Err(CapsuleError::Invalid(
                        "timing diagnostics have an invalid sample".into(),
                    ));
                }
            }
            // The flag list must be exactly the flagged samples, in order.
            let expected_outliers: Vec<u32> = timing
                .samples
                .iter()
                .filter(|observation| observation.outlier)
                .map(|observation| observation.run)
                .collect();
            if timing.outlier_runs != expected_outliers {
                return Err(CapsuleError::Invalid(
                    "timing outlier_runs disagrees with the flagged samples".into(),
                ));
            }
            // A recorded outlier must follow from the median and the factors,
            // so the flags cannot be arbitrary.
            let values: Vec<f64> = timing
                .samples
                .iter()
                .map(|observation| observation.wall_ns)
                .collect();
            let median = repeatability::median(&values).unwrap_or(0.0);
            for observation in &timing.samples {
                let expected = if median > 0.0 && observation.wall_ns > median * timing.slow_factor
                {
                    Some("slow")
                } else if median > 0.0 && observation.wall_ns < median * timing.fast_factor {
                    Some("fast")
                } else {
                    None
                };
                if observation.direction.as_deref() != expected {
                    return Err(CapsuleError::Invalid(
                        "timing outlier flag does not follow from its median and factors".into(),
                    ));
                }
            }
            if timing
                .trimmed_median_ns
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                return Err(CapsuleError::Invalid(
                    "timing trimmed_median_ns is not a valid duration".into(),
                ));
            }
            let warm_up = &timing.warm_up;
            if warm_up.leading_slow_runs as usize > timing.samples.len()
                || !warm_up.first_run_penalty_pct.is_finite()
                || warm_up.first_run_penalty_pct < 0.0
            {
                return Err(CapsuleError::Invalid(
                    "timing warm_up record is inconsistent".into(),
                ));
            }
            // The leading-slow-run count must be exactly the run of slow flags
            // at the start of the sequence, so it cannot be arbitrary.
            let expected_leading = timing
                .samples
                .iter()
                .take_while(|observation| observation.direction.as_deref() == Some("slow"))
                .count() as u32;
            if warm_up.leading_slow_runs != expected_leading {
                return Err(CapsuleError::Invalid(
                    "timing warm_up leading_slow_runs disagrees with the samples".into(),
                ));
            }
            // The first-run penalty must follow from the first sample and the
            // median, so it cannot be arbitrary either.
            let expected_penalty = if median > 0.0 {
                let first = timing.samples.first().map(|o| o.wall_ns).unwrap_or(0.0);
                ((first - median) / median * 100.0).max(0.0)
            } else {
                0.0
            };
            if (warm_up.first_run_penalty_pct - expected_penalty).abs() > 1e-9 {
                return Err(CapsuleError::Invalid(
                    "timing warm_up first_run_penalty_pct disagrees with the samples".into(),
                ));
            }
        }

        let reference_wall_clock = self
            .metrics
            .iter()
            .find(|metric| metric.name == "reference_wall_clock");
        let paired_speedup_ratio = self
            .metrics
            .iter()
            .find(|metric| metric.name == "paired_speedup_ratio");
        if reference_wall_clock.is_some() != paired_speedup_ratio.is_some() {
            return Err(CapsuleError::Invalid(
                "reference_wall_clock and paired_speedup_ratio must be recorded together".into(),
            ));
        }
        if let (Some(reference), Some(ratio)) = (reference_wall_clock, paired_speedup_ratio) {
            if reference.kind != MetricKind::Indicative
                || reference.method != MetricMethod::WallClock
                || reference.unit != "nanoseconds"
                || ratio.kind != MetricKind::Indicative
                || ratio.method != MetricMethod::WallClock
                || ratio.unit != "ratio"
                || ratio.value <= 0.0
            {
                return Err(CapsuleError::Invalid(
                    "paired reference metrics have invalid labels, units, or values".into(),
                ));
            }
            match &self.reference_build {
                Some(reference_build) => {
                    let digest = reference_build
                        .binary_sha256
                        .strip_prefix("sha256:")
                        .unwrap_or("");
                    if digest.len() != 64
                        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                        || reference_build.argv.is_empty()
                        || reference_build
                            .argv
                            .iter()
                            .any(|argument| argument.contains('\0'))
                        || !self.source.files.contains_key("problem/reference.cpp")
                    {
                        return Err(CapsuleError::Invalid(
                            "paired reference build metadata is incomplete or invalid".into(),
                        ));
                    }
                }
                None if self.harness.version == "0.12.0" => {}
                None => {
                    return Err(CapsuleError::Invalid(
                        "paired metrics require reference build metadata".into(),
                    ))
                }
            }
        } else if self.reference_build.is_some() {
            return Err(CapsuleError::Invalid(
                "reference build metadata requires paired timing metrics".into(),
            ));
        }

        if let Some(repeatability) = &self.repeatability {
            if repeatability.runs < 2 {
                return Err(CapsuleError::Invalid(
                    "repeatability.runs must be at least 2".into(),
                ));
            }
            if repeatability.runs > 100 {
                return Err(CapsuleError::Invalid(
                    "repeatability.runs must not exceed 100".into(),
                ));
            }
            if self.source.language == Language::Go && repeatability.runs < 20 {
                return Err(CapsuleError::Invalid(
                    "Go capsules require at least 20 repeatability runs".into(),
                ));
            }
            for (name, spread) in &repeatability.per_metric {
                if spread.samples != repeatability.runs
                    || spread.samples > 100
                    || !spread.min.is_finite()
                    || !spread.max.is_finite()
                    || !spread.median.is_finite()
                    || !spread.spread_pct.is_finite()
                    || spread.min > spread.median
                    || spread.median > spread.max
                    || spread.spread_pct < 0.0
                    || spread.p95.is_some_and(|value| {
                        !value.is_finite() || value < spread.median || value > spread.max
                    })
                    || spread.p99.is_some_and(|value| {
                        !value.is_finite() || value < spread.median || value > spread.max
                    })
                    || matches!((spread.p95, spread.p99), (Some(p95), Some(p99)) if p99 < p95)
                    || !spread.matches_values()
                    || (self.harness.version == HARNESS_VERSION && spread.values.is_none())
                {
                    return Err(CapsuleError::Invalid(format!(
                        "repeatability metric {name:?} has invalid spread"
                    )));
                }
            }
        }

        if self.build.build_repeat == 0 || self.build.build_repeat > 20 {
            return Err(CapsuleError::Invalid(
                "build_repeat must be between 1 and 20".into(),
            ));
        }

        match (&self.build_repeatability, self.build.build_repeat) {
            (Some(repeatability), runs) => {
                if runs < 2 {
                    return Err(CapsuleError::Invalid(
                        "build_repeatability requires at least two builds".into(),
                    ));
                }
                if repeatability.runs != runs {
                    return Err(CapsuleError::Invalid(
                        "build_repeatability.runs must equal build.build_repeat".into(),
                    ));
                }
                for name in repeatability.per_metric.keys() {
                    if !matches!(
                        name.as_str(),
                        "build_wall_clock" | "build_user_cpu_time" | "build_system_cpu_time"
                    ) {
                        return Err(CapsuleError::Invalid(format!(
                            "unknown build repeatability metric {name:?}"
                        )));
                    }
                    if !self.metrics.iter().any(|metric| metric.name == *name) {
                        return Err(CapsuleError::Invalid(format!(
                            "build repeatability metric {name:?} has no corresponding metric"
                        )));
                    }
                }
                for (name, spread) in &repeatability.per_metric {
                    if spread.samples != runs
                        || !spread.min.is_finite()
                        || !spread.max.is_finite()
                        || !spread.median.is_finite()
                        || !spread.spread_pct.is_finite()
                        || spread.min > spread.median
                        || spread.median > spread.max
                        || spread.spread_pct < 0.0
                        || spread.p95.is_some_and(|value| {
                            !value.is_finite() || value < spread.median || value > spread.max
                        })
                        || spread.p99.is_some_and(|value| {
                            !value.is_finite() || value < spread.median || value > spread.max
                        })
                        || matches!((spread.p95, spread.p99), (Some(p95), Some(p99)) if p99 < p95)
                        || !spread.matches_values()
                        || (self.harness.version == HARNESS_VERSION && spread.values.is_none())
                    {
                        return Err(CapsuleError::Invalid(format!(
                            "build repeatability metric {name:?} has invalid spread"
                        )));
                    }
                    let metric = self
                        .metrics
                        .iter()
                        .find(|metric| metric.name == *name)
                        .unwrap();
                    if metric.value != spread.median
                        || metric.uncertainty.as_ref().is_none_or(|uncertainty| {
                            uncertainty.kind != UncertaintyKind::Ci95
                                || !uncertainty.value.is_finite()
                                || uncertainty.value < 0.0
                        })
                    {
                        return Err(CapsuleError::Invalid(format!(
                            "build metric {name:?} does not match its repeatability summary"
                        )));
                    }
                }
                if !repeatability.per_metric.contains_key("build_wall_clock") {
                    return Err(CapsuleError::Invalid(
                        "build repeatability must include build_wall_clock".into(),
                    ));
                }
            }
            (None, 1) => {}
            (None, _) => {
                return Err(CapsuleError::Invalid(
                    "repeated builds require build_repeatability data".into(),
                ))
            }
        }

        let id = self
            .capsule_id
            .as_ref()
            .ok_or_else(|| CapsuleError::Invalid("capsule_id is missing".into()))?;
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CapsuleError::Invalid(
                "capsule_id must be a 64-character SHA-256 digest".into(),
            ));
        }
        let computed = self.compute_id()?;
        if &computed != id {
            return Err(CapsuleError::IdMismatch {
                claimed: id.clone(),
                computed,
            });
        }

        Ok(())
    }

    pub fn to_json_pretty(&self) -> Result<String, CapsuleError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(text: &str) -> Result<Self, CapsuleError> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let version = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| CapsuleError::Invalid("missing schema_version".into()))?;
        let version = u32::try_from(version)
            .map_err(|_| CapsuleError::Invalid("schema_version is out of range".into()))?;
        let migrated = version != CURRENT_SCHEMA_VERSION;
        let mut capsule: Capsule = if !migrated {
            serde_json::from_value(value)?
        } else {
            let migrated = migrate::migrate_value(value, version)?;
            serde_json::from_value(migrated)?
        };
        if migrated {
            capsule.capsule_id = None;
            capsule.seal()?;
        }
        capsule.validate()?;
        Ok(capsule)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// `source_sha256`: over `"<path>\0<hex>\n"` for each entry, paths sorted.
pub fn hash_source_files(files: &BTreeMap<String, String>) -> String {
    let mut buf = String::new();
    for (path, hash) in files {
        let hex = hash.strip_prefix("sha256:").unwrap_or(hash);
        buf.push_str(path);
        buf.push('\0');
        buf.push_str(hex);
        buf.push('\n');
    }
    sha256_hex(buf.as_bytes())
}

pub fn host_fingerprint(cpu_model: &str, arch: &str, os: &str, kernel: &str, nproc: u32) -> String {
    sha256_hex(format!("{cpu_model}|{arch}|{os}|{kernel}|{nproc}").as_bytes())
}

pub fn fixed_env_map() -> BTreeMap<String, String> {
    FIXED_ENV
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getrusage_metrics_are_machine_dependent_even_when_measured() {
        let counter = Metric::count(
            "user_cpu_time",
            10,
            MetricKind::Measured,
            MetricMethod::Getrusage,
        );
        let deterministic = Metric::count(
            "allocations",
            10,
            MetricKind::Measured,
            MetricMethod::ValgrindMemcheck,
        );
        let hardware = Metric::count(
            "instructions_hw",
            10,
            MetricKind::Measured,
            MetricMethod::Perf,
        );
        assert!(counter.machine_dependent());
        assert!(hardware.machine_dependent());
        assert!(!deterministic.machine_dependent());
    }

    #[test]
    fn wall_clock_metrics_are_machine_dependent_even_when_measured() {
        let build_time = Metric::count(
            "build_wall_clock",
            10,
            MetricKind::Measured,
            MetricMethod::WallClock,
        );
        assert!(build_time.machine_dependent());
    }

    #[test]
    fn golden_capsule_round_trips_with_its_identity() {
        let capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v2.json")).unwrap();
        assert_eq!(capsule.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            capsule.capsule_id.as_deref(),
            Some(capsule.compute_id().unwrap().as_str())
        );
        assert!(capsule
            .metrics
            .iter()
            .any(|metric| metric.name == "wall_clock"));
    }

    #[test]
    fn comparison_context_rejects_changed_build_profiles_but_allows_source_layout_changes() {
        let before =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v11.json")).unwrap();
        let mut after = before.clone();
        after.build.argv.push("/src0/extra.cpp".into());
        assert!(before.same_comparison_context(&after));

        after.build.opt_level = "-O3".into();
        assert!(!before.same_comparison_context(&after));

        after = before.clone();
        after.build.argv.push("-march=native".into());
        assert!(!before.same_comparison_context(&after));

        after = before.clone();
        after
            .source
            .files
            .insert("CMakeLists.txt".into(), "sha256:changed".into());
        assert!(!before.same_comparison_context(&after));
    }

    #[test]
    fn schema_eleven_repeatability_capsule_round_trips_with_tail_quantiles() {
        let json = include_str!("../tests/fixtures/capsule-v11.json");
        let capsule = Capsule::from_json(json).unwrap();
        assert_eq!(capsule.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            capsule.build.binary_sha256,
            LEGACY_BINARY_DIGEST_UNAVAILABLE
        );
        assert_eq!(capsule.repeatability.as_ref().unwrap().runs, 20);
        let instructions = &capsule.repeatability.as_ref().unwrap().per_metric["instructions"];
        assert_eq!(instructions.min, instructions.max);
        assert_eq!(instructions.p95, Some(instructions.median));
        assert_eq!(instructions.p99, Some(instructions.median));
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(decoded, capsule);
    }

    #[test]
    fn schema_twelve_capsule_round_trips_with_executable_digest() {
        let json = include_str!("../tests/fixtures/capsule-v12.json");
        let capsule = Capsule::from_json(json).unwrap();
        assert_eq!(capsule.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(capsule.build.binary_sha256.starts_with("sha256:"));
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(decoded, capsule);
    }

    #[test]
    fn schema_fifteen_golden_capsule_round_trips_with_raw_samples() {
        let json = include_str!("../tests/fixtures/capsule-v15.json");
        let capsule = Capsule::from_json(json).unwrap();
        assert_eq!(capsule.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(capsule.harness.version, HARNESS_VERSION);
        assert_eq!(capsule.repeatability.as_ref().unwrap().runs, 20);
        assert_eq!(
            capsule.repeatability.as_ref().unwrap().per_metric["instructions"]
                .values
                .as_ref()
                .unwrap()
                .len(),
            20
        );
        assert_eq!(capsule.build_repeatability.as_ref().unwrap().runs, 3);
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(decoded, capsule);
    }

    #[test]
    fn published_pair_counting_capsules_preserve_the_documented_comparison() {
        let naive = Capsule::from_json(include_str!(
            "../../../evidence/sorted-pair-sum-naive-v0.19.json"
        ))
        .unwrap();
        let good = Capsule::from_json(include_str!(
            "../../../evidence/sorted-pair-sum-good-v0.19.json"
        ))
        .unwrap();

        assert!(naive.same_comparison_context(&good));
        assert_eq!(naive.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(good.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(naive.harness.version, HARNESS_VERSION);
        assert_eq!(good.harness.version, HARNESS_VERSION);
        assert_eq!(naive.source.language, Language::Cpp);
        assert_eq!(good.source.language, Language::Cpp);
        assert_eq!(naive.input.seed, 1);
        assert_eq!(good.input.seed, 1);
        assert_eq!(naive.input.size, Some(800));
        assert_eq!(good.input.size, Some(800));
        assert_eq!(naive.repeatability.as_ref().unwrap().runs, 20);
        assert_eq!(good.repeatability.as_ref().unwrap().runs, 20);
        assert_eq!(naive.build_repeatability.as_ref().unwrap().runs, 3);
        assert_eq!(good.build_repeatability.as_ref().unwrap().runs, 3);

        let expected = [
            (
                "instructions",
                5_317_517.0,
                2_506_969.0,
                MetricKind::Simulated,
            ),
            (
                "memory_refs",
                6_520_988.0,
                3_409_423.0,
                MetricKind::Simulated,
            ),
            ("branches", 714_278.0, 407_255.0, MetricKind::Simulated),
            ("branch_misses", 15_727.0, 18_868.0, MetricKind::Simulated),
            ("cache_misses", 17_683.0, 17_708.0, MetricKind::Simulated),
            ("allocations", 4.0, 4.0, MetricKind::Measured),
            ("allocated_bytes", 88_320.0, 88_320.0, MetricKind::Measured),
            ("binary_size", 16_560.0, 16_904.0, MetricKind::Measured),
        ];

        for (name, naive_value, good_value, kind) in expected {
            let naive_metric = naive
                .metrics
                .iter()
                .find(|metric| metric.name == name)
                .unwrap();
            let good_metric = good
                .metrics
                .iter()
                .find(|metric| metric.name == name)
                .unwrap();
            assert_eq!(naive_metric.value, naive_value);
            assert_eq!(good_metric.value, good_value);
            assert_eq!(naive_metric.kind, kind);
            assert_eq!(good_metric.kind, kind);

            for capsule in [&naive, &good] {
                let values = capsule.repeatability.as_ref().unwrap().per_metric[name]
                    .values
                    .as_ref()
                    .unwrap();
                assert_eq!(values.len(), 20);
                assert!(values.iter().all(|value| *value
                    == capsule
                        .metrics
                        .iter()
                        .find(|metric| metric.name == name)
                        .unwrap()
                        .value));
            }
        }
    }

    #[test]
    fn schema_fifteen_go_capsule_preserves_unrankable_raw_samples() {
        let json = include_str!("../tests/fixtures/go-capsule-v15.json");
        let capsule = Capsule::from_json(json).unwrap();
        assert_eq!(capsule.source.language, Language::Go);
        let spread = &capsule.repeatability.as_ref().unwrap().per_metric["instructions"];
        assert_eq!(spread.values.as_ref().unwrap().len(), 20);
        assert!(spread.spread_pct > repeatability::DEFAULT_RANKING_SPREAD_TOLERANCE_PCT);
        let metric = capsule
            .metrics
            .iter()
            .find(|metric| metric.name == "instructions")
            .unwrap();
        let eligibility = repeatability::ranking_eligibility(Some(metric), Some(spread));
        assert!(!eligibility.eligible);
    }

    #[test]
    fn schema_thirteen_paired_capsule_round_trips_reference_provenance() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.schema_version = CURRENT_SCHEMA_VERSION;
        capsule.build.build_repeat = 1;
        capsule.metrics.push(Metric {
            name: "reference_wall_clock".into(),
            value: 2_000_000.0,
            unit: "nanoseconds".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 10_000.0,
            }),
        });
        capsule.metrics.push(Metric {
            name: "paired_speedup_ratio".into(),
            value: 2.0,
            unit: "ratio".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 0.02,
            }),
        });
        capsule.reference_build = Some(ReferenceBuild {
            binary_sha256: format!("sha256:{}", "b".repeat(64)),
            argv: vec![
                "g++".into(),
                "-O2".into(),
                "-std=c++20".into(),
                "/src0/reference.cpp".into(),
            ],
        });
        capsule.seal().unwrap();
        assert!(capsule.validate().is_ok());
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(decoded, capsule);
    }

    #[test]
    fn schema_twelve_requires_the_binary_digest_field() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        value["build"]
            .as_object_mut()
            .unwrap()
            .remove("binary_sha256");
        assert!(matches!(
            Capsule::from_json(&value.to_string()),
            Err(CapsuleError::Json(_))
        ));
    }

    #[test]
    fn paired_wall_clock_metrics_require_a_complete_indicative_pair() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.harness.version = "0.12.0".into();
        capsule.metrics.push(Metric {
            name: "reference_wall_clock".into(),
            value: 2_000_000.0,
            unit: "nanoseconds".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 10_000.0,
            }),
        });
        capsule.metrics.push(Metric {
            name: "paired_speedup_ratio".into(),
            value: 2.0,
            unit: "ratio".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 0.02,
            }),
        });
        capsule.seal().unwrap();
        assert!(capsule.validate().is_ok());

        capsule.metrics.pop();
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn paired_speedup_ratio_rejects_invalid_labels_and_values() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.metrics.push(Metric {
            name: "reference_wall_clock".into(),
            value: 2_000_000.0,
            unit: "nanoseconds".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 10_000.0,
            }),
        });
        capsule.metrics.push(Metric {
            name: "paired_speedup_ratio".into(),
            value: 0.0,
            unit: "ratio".into(),
            kind: MetricKind::Indicative,
            method: MetricMethod::WallClock,
            uncertainty: Some(Uncertainty {
                kind: UncertaintyKind::Ci95,
                value: 0.02,
            }),
        });
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn binary_digest_must_be_a_sha256_digest() {
        let json = include_str!("../tests/fixtures/capsule-v11.json");
        let mut capsule = Capsule::from_json(json).unwrap();
        capsule.build.binary_sha256 = format!("sha256:{}", "a".repeat(64));
        capsule.seal().unwrap();
        assert!(capsule.validate().is_ok());
        capsule.build.binary_sha256 = "sha256:invalid".into();
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn repeatability_sample_count_must_match_run_count() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        let repeatability = capsule.repeatability.as_mut().unwrap();
        repeatability
            .per_metric
            .values_mut()
            .next()
            .unwrap()
            .samples -= 1;
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn current_harness_requires_raw_samples_and_checks_their_summary() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.harness.version = HARNESS_VERSION.into();
        let repeatability = capsule.repeatability.as_mut().unwrap();
        for spread in repeatability.per_metric.values_mut() {
            let values = vec![spread.median; spread.samples as usize];
            *spread = repeatability::spread(&values).unwrap();
        }
        capsule.seal().unwrap();
        assert!(capsule.validate().is_ok());

        let mut missing = capsule.clone();
        missing
            .repeatability
            .as_mut()
            .unwrap()
            .per_metric
            .values_mut()
            .next()
            .unwrap()
            .values = None;
        missing.seal().unwrap();
        assert!(matches!(missing.validate(), Err(CapsuleError::Invalid(_))));

        let mut inconsistent = capsule;
        inconsistent
            .repeatability
            .as_mut()
            .unwrap()
            .per_metric
            .values_mut()
            .next()
            .unwrap()
            .values
            .as_mut()
            .unwrap()[0] += 1.0;
        inconsistent.seal().unwrap();
        assert!(matches!(
            inconsistent.validate(),
            Err(CapsuleError::Invalid(_))
        ));
    }

    #[test]
    fn repeated_build_capsules_require_a_matching_summary() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.build.build_repeat = 3;
        capsule.build_repeatability = Some(Repeatability {
            runs: 2,
            per_metric: Default::default(),
        });
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn capsule_cannot_request_more_than_twenty_clean_builds() {
        let mut capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v12.json")).unwrap();
        capsule.build.build_repeat = 21;
        capsule.seal().unwrap();
        assert!(matches!(capsule.validate(), Err(CapsuleError::Invalid(_))));
    }

    #[test]
    fn go_capsule_v7_round_trips_with_its_identity() {
        let json = include_str!("../tests/fixtures/go-capsule-v7.json");
        let capsule = Capsule::from_json(json).unwrap();
        assert_eq!(capsule.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(capsule.source.language, Language::Go);
        assert_eq!(capsule.repeatability.as_ref().unwrap().runs, 20);
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(decoded, capsule);
    }

    #[test]
    fn unknown_nested_capsule_fields_are_rejected() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/capsule-v2.json")).unwrap();
        value["toolchain"]["unrecognized"] = serde_json::json!("value");
        let result = Capsule::from_json(&value.to_string());
        assert!(matches!(result, Err(CapsuleError::Json(_))));
    }

    #[test]
    fn timestamp_formats_unix_epoch_as_utc() {
        assert_eq!(format_utc_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn tolerance_policy_matches_only_its_toolchain_and_metrics() {
        let policy = tolerance_policy_for("gdc", "instructions").expect("gdc policy exists");
        assert_eq!(policy.id, "d-gdc-druntime-cpuid");
        assert!(policy.relative_tolerance > 0.0);
        assert!(policy.metrics.contains(&"instructions"));
        assert!(tolerance_policy_for("gcc", "instructions").is_none());
        assert!(tolerance_policy_for("gdc", "not_a_metric").is_none());
    }

    #[test]
    fn every_policy_bound_covers_the_largest_observed_host_offset() {
        // The hosted runners have shown at most 0.043% on any single simulated
        // metric (C++/Rust `branch_misses`). Every policy must cover that, or
        // the cross-machine gate flakes whenever an Intel host joins the pool.
        for policy in TOLERANCE_POLICIES {
            assert!(
                policy.relative_tolerance >= 1e-3,
                "{} bound {} is below the observed host offset",
                policy.id,
                policy.relative_tolerance
            );
            assert_eq!(policy.metrics, SIMULATED_METRICS);
        }
    }

    #[test]
    fn rust_has_a_policy_because_its_ifunc_dispatch_is_cpu_model_dependent() {
        let policy = tolerance_policy_for("rust", "branch_misses").expect("rust policy exists");
        assert_eq!(policy.id, "rust-libc-ifunc-dispatch");
        assert!(policy.metrics.contains(&"branch_instructions"));
        // The policy must never leak onto a measured metric.
        assert!(tolerance_policy_for("rust", "allocations").is_none());
    }

    #[test]
    fn applied_tolerance_records_only_covered_metrics() {
        let spreads = vec![
            ("instructions".to_string(), 1.5e-7),
            ("user_cpu_time".to_string(), 0.5),
        ];
        let applied = AppliedTolerance::record("gdc", &spreads).expect("policy applies to gdc");
        assert_eq!(applied.policy, "d-gdc-druntime-cpuid");
        assert_eq!(applied.metrics, vec!["instructions".to_string()]);
        assert_eq!(applied.observed_relative_spread, 1.5e-7);
        assert!(AppliedTolerance::record("gcc", &spreads).is_none());
    }

    #[test]
    fn applied_tolerance_is_validated_against_its_policy() {
        let capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v15.json")).unwrap();
        let with_tolerance = |tolerance: AppliedTolerance| {
            let mut candidate = capsule.clone();
            candidate.applied_tolerance = Some(tolerance);
            candidate.seal().unwrap();
            candidate
        };

        let valid = with_tolerance(AppliedTolerance {
            policy: "d-gdc-druntime-cpuid".into(),
            toolchain: "gdc".into(),
            relative_tolerance: 1e-5,
            metrics: vec!["instructions".into()],
            observed_relative_spread: 2.0e-6,
        });
        assert!(valid.validate().is_ok(), "{:?}", valid.validate());

        // An unknown policy is rejected.
        assert!(with_tolerance(AppliedTolerance {
            policy: "does-not-exist".into(),
            toolchain: "gdc".into(),
            relative_tolerance: 1e-5,
            metrics: vec!["instructions".into()],
            observed_relative_spread: 0.0,
        })
        .validate()
        .is_err());

        // A tolerance wider than the declared bound is rejected.
        assert!(with_tolerance(AppliedTolerance {
            policy: "d-gdc-druntime-cpuid".into(),
            toolchain: "gdc".into(),
            relative_tolerance: 1.0,
            metrics: vec!["instructions".into()],
            observed_relative_spread: 0.0,
        })
        .validate()
        .is_err());

        // A policy may not cover a measured metric.
        assert!(with_tolerance(AppliedTolerance {
            policy: "d-gdc-druntime-cpuid".into(),
            toolchain: "gdc".into(),
            relative_tolerance: 1e-5,
            metrics: vec!["allocations".into()],
            observed_relative_spread: 0.0,
        })
        .validate()
        .is_err());

        // A policy may not name a metric the capsule does not record.
        let mut missing = with_tolerance(AppliedTolerance {
            policy: "d-gdc-druntime-cpuid".into(),
            toolchain: "gdc".into(),
            relative_tolerance: 1e-5,
            metrics: vec!["branches".into()],
            observed_relative_spread: 0.0,
        });
        missing.metrics.retain(|metric| metric.name != "branches");
        missing.seal().unwrap();
        assert!(missing.validate().is_err());
    }

    #[test]
    fn noise_canary_status_follows_its_own_drift() {
        let stable = NoiseCanary::from_samples("ccx-canary-v1", 1_000_000.0, 1_050_000.0);
        assert_eq!(stable.status, "stable");
        assert!(stable.is_stable());
        assert!((stable.drift_pct - 5.0).abs() < 1e-9);
        assert!(stable.note.is_none());

        let drifted = NoiseCanary::from_samples("ccx-canary-v1", 1_000_000.0, 2_000_000.0);
        assert_eq!(drifted.status, "drifted");
        assert!(!drifted.is_stable());
        assert!((drifted.drift_pct - 100.0).abs() < 1e-9);
    }

    #[test]
    fn noise_canary_drift_is_symmetric_in_its_samples() {
        assert_eq!(
            relative_drift_pct(1_000.0, 500.0),
            relative_drift_pct(500.0, 1_000.0)
        );
        assert_eq!(relative_drift_pct(0.0, 500.0), 0.0);
    }

    #[test]
    fn noise_canary_validation_rejects_inconsistent_records() {
        let capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v15.json")).unwrap();
        let with_canary = |canary: NoiseCanary| {
            let mut candidate = capsule.clone();
            candidate.noise_canary = Some(canary);
            candidate.seal().unwrap();
            candidate
        };

        // A well-formed, self-consistent canary is accepted.
        let valid = with_canary(NoiseCanary::from_samples(
            "ccx-canary-v1",
            1_000_000.0,
            1_100_000.0,
        ));
        assert!(valid.validate().is_ok(), "{:?}", valid.validate());

        // A status that disagrees with the recorded drift is rejected.
        assert!(with_canary(NoiseCanary {
            workload: "ccx-canary-v1".into(),
            before_wall_ns: 1_000_000.0,
            after_wall_ns: 2_000_000.0,
            drift_pct: 100.0,
            tolerance_pct: 20.0,
            status: "stable".into(),
            note: None,
        })
        .validate()
        .is_err());

        // A drift that disagrees with the samples is rejected.
        assert!(with_canary(NoiseCanary {
            workload: "ccx-canary-v1".into(),
            before_wall_ns: 1_000_000.0,
            after_wall_ns: 1_000_000.0,
            drift_pct: 50.0,
            tolerance_pct: 20.0,
            status: "drifted".into(),
            note: None,
        })
        .validate()
        .is_err());

        // An unavailable canary without a reason is rejected.
        assert!(with_canary(NoiseCanary {
            workload: "ccx-canary-v1".into(),
            before_wall_ns: 0.0,
            after_wall_ns: 0.0,
            drift_pct: 0.0,
            tolerance_pct: 20.0,
            status: "unavailable".into(),
            note: None,
        })
        .validate()
        .is_err());

        // An unknown status is rejected.
        assert!(with_canary(NoiseCanary {
            workload: "ccx-canary-v1".into(),
            before_wall_ns: 1.0,
            after_wall_ns: 1.0,
            drift_pct: 0.0,
            tolerance_pct: 20.0,
            status: "vibes".into(),
            note: None,
        })
        .validate()
        .is_err());
    }

    #[test]
    fn timing_flags_slow_and_fast_outliers_against_the_median() {
        // Median 100; 500 is >2x (slow), 40 is <0.5x (fast).
        let diagnostic = TimingDiagnostic::analyse(&[100, 100, 100, 500, 40]);
        assert_eq!(diagnostic.outlier_runs, vec![3, 4]);
        assert_eq!(diagnostic.samples[3].direction.as_deref(), Some("slow"));
        assert_eq!(diagnostic.samples[4].direction.as_deref(), Some("fast"));
        assert!(diagnostic.samples[0].direction.is_none());
        // The trimmed median drops the outliers and keeps the clean samples.
        assert_eq!(diagnostic.trimmed_median_ns, Some(100.0));
    }

    #[test]
    fn timing_trims_only_when_enough_clean_samples_remain() {
        // Two clean samples are too few to call a trimmed median a median.
        let diagnostic = TimingDiagnostic::analyse(&[100, 100, 10_000]);
        assert_eq!(diagnostic.outlier_runs, vec![2]);
        assert_eq!(diagnostic.trimmed_median_ns, None);
    }

    #[test]
    fn warm_up_counts_only_leading_slow_runs() {
        // 10000 and 10000 are leading slow runs; the later 10000 is slow too but
        // not leading, so it does not count toward warm-up.
        let diagnostic = TimingDiagnostic::analyse(&[10_000, 10_000, 100, 100, 100, 100, 10_000]);
        assert_eq!(diagnostic.outlier_runs, vec![0, 1, 6]);
        assert_eq!(diagnostic.warm_up.leading_slow_runs, 2);
        // The first run is 9900% above the median of 100.
        assert!((diagnostic.warm_up.first_run_penalty_pct - 9900.0).abs() < 1e-6);

        // A settled series has no leading slow runs and no first-run penalty.
        let settled = TimingDiagnostic::analyse(&[100, 101, 99, 100]);
        assert_eq!(settled.warm_up.leading_slow_runs, 0);
        assert_eq!(settled.warm_up.first_run_penalty_pct, 0.0);
    }

    #[test]
    fn timing_validation_rejects_inconsistent_records() {
        let capsule =
            Capsule::from_json(include_str!("../tests/fixtures/capsule-v15.json")).unwrap();
        let with_timing = |timing: TimingDiagnostic| {
            let mut candidate = capsule.clone();
            candidate.timing = Some(timing);
            candidate.seal().unwrap();
            candidate
        };

        // A well-formed diagnostic is accepted.
        let valid = with_timing(TimingDiagnostic::analyse(&[100, 100, 100, 500, 40]));
        assert!(valid.validate().is_ok(), "{:?}", valid.validate());

        // A flag that does not follow from the median is rejected.
        let mut bogus = TimingDiagnostic::analyse(&[100, 100, 100, 500]);
        bogus.samples[0].outlier = true;
        bogus.samples[0].direction = Some("slow".into());
        bogus.outlier_runs = vec![0, 3];
        assert!(with_timing(bogus).validate().is_err());

        // An outlier_runs list that disagrees with the samples is rejected.
        let mut mismatched = TimingDiagnostic::analyse(&[100, 100, 100, 500]);
        mismatched.outlier_runs = vec![];
        assert!(with_timing(mismatched).validate().is_err());

        // A run index that is out of order is rejected.
        let mut disordered = TimingDiagnostic::analyse(&[100, 100, 100, 500]);
        disordered.samples[1].run = 9;
        assert!(with_timing(disordered).validate().is_err());

        // An unknown direction is rejected.
        let mut unknown = TimingDiagnostic::analyse(&[100, 100, 100, 500]);
        unknown.samples[3].direction = Some("sideways".into());
        assert!(with_timing(unknown).validate().is_err());

        // A leading-slow-run count that disagrees with the flags is rejected.
        let mut wrong_warm_up = TimingDiagnostic::analyse(&[10_000, 100, 100, 100]);
        wrong_warm_up.warm_up.leading_slow_runs = 3;
        assert!(with_timing(wrong_warm_up).validate().is_err());

        // A first-run penalty that disagrees with the samples is rejected.
        let mut wrong_penalty = TimingDiagnostic::analyse(&[100, 100, 100, 100]);
        wrong_penalty.warm_up.first_run_penalty_pct = 5.0;
        assert!(with_timing(wrong_penalty).validate().is_err());
    }
}
