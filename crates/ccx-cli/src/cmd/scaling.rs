//! `ccx scaling` — does more parallelism actually pay?
//!
//! This runs the same solution, on the same input, at a sweep of thread counts
//! and reports the speed-up against the one-thread baseline and the efficiency
//! (speed-up per thread). It is the answer to "should I add threads", and the
//! honest part is what it refuses to do: it never extrapolates a thread count it
//! did not run, and it never presents a wall-clock ratio as deterministic.
//!
//! Every number is `indicative`. Speed-up is a ratio of whole-process wall
//! times, so it moves with the machine, its load, and its core count. A serial
//! solution produces a flat curve, and saying so is the correct result.

use crate::cmd::{self, Fixable};
use crate::context::{policy_for, Solution};
use crate::pipeline::Pipeline;
use crate::Format;
use ccx_core::capsule::Verdict;
use ccx_core::problem::Judge;
use ccx_core::scaling::{self, ScalingCurve};
use serde::Serialize;
use std::path::Path;

/// The default tolerance for "speed-up stopped growing": a further point must
/// improve speed-up by more than this fraction to count as still scaling.
pub const DEFAULT_TOLERANCE: f64 = 0.05;

#[derive(Debug, Clone, Serialize)]
pub struct ThreadSamples {
    pub threads: u32,
    /// Every wall time observed at this thread count, in nanoseconds.
    pub samples_ns: Vec<u64>,
    /// Median wall time, in nanoseconds.
    pub median_ns: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScalingReport {
    pub problem: String,
    pub language: String,
    pub size: String,
    pub seed: u64,
    /// The environment variable the sweep set to choose the thread count.
    pub thread_env: String,
    /// Always `indicative`: speed-up is a wall-clock ratio.
    pub kind: String,
    pub interpretation: String,
    pub curve: ScalingCurve,
    pub runs: Vec<ThreadSamples>,
    pub warnings: Vec<String>,
}

/// The default thread sweep: powers of two up to the host's logical CPU count.
pub fn default_threads(host_cpus: u32) -> Vec<u32> {
    let mut threads = vec![1];
    let mut next = 2;
    while next <= host_cpus.max(1) {
        threads.push(next);
        next *= 2;
    }
    threads
}

/// Parse `--threads 1,2,4` into an ordered, de-duplicated sweep.
pub fn parse_threads(spec: &str) -> Result<Vec<u32>, String> {
    let mut threads = Vec::new();
    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let value: u32 = token
            .parse()
            .map_err(|_| format!("{token:?} is not a thread count"))?;
        if value == 0 {
            return Err("thread counts must be at least 1".to_string());
        }
        if value > 256 {
            return Err(format!(
                "thread count {value} is above the supported maximum of 256"
            ));
        }
        if !threads.contains(&value) {
            threads.push(value);
        }
    }
    if threads.is_empty() {
        return Err("no thread counts were given".to_string());
    }
    threads.sort_unstable();
    Ok(threads)
}

/// Set `VAR=<threads>` on a program so the process actually sees it.
///
/// A program carries two command prefixes: `runtime_prefix`, which holds the
/// toolchain's runtime environment (the Go profile pins `GOMAXPROCS=1` there),
/// and `prefix`, which is where an instrument such as Valgrind goes. Both end up
/// in the same command, so the later one wins. The assignment must therefore go
/// into whichever prefix already runs `env`, replacing any existing assignment
/// of the same variable — otherwise the inner `env` would shadow the sweep and
/// every thread count would run pinned to one thread.
pub fn with_thread_env(
    program: &mut crate::pipeline::Program,
    var: &str,
    threads: u32,
) -> anyhow::Result<()> {
    if !is_env_name(var) {
        return Err(
            Fixable::new(format!("{var:?} is not a valid environment variable name"))
                .fix("pass --thread-env with a name like OMP_NUM_THREADS or GOMAXPROCS")
                .into_error(),
        );
    }
    let assignment = format!("{var}={threads}");
    let target = if program.runtime_prefix.first().map(String::as_str) == Some("env") {
        &mut program.runtime_prefix
    } else if program.prefix.first().map(String::as_str) == Some("env") {
        &mut program.prefix
    } else {
        let mut prefix = vec!["env".to_string(), assignment];
        prefix.append(&mut program.prefix);
        program.prefix = prefix;
        return Ok(());
    };
    if let Some(index) = target
        .iter()
        .position(|token| token.starts_with(&format!("{var}=")))
    {
        target[index] = assignment;
    } else {
        target.insert(1, assignment);
    }
    Ok(())
}

/// A POSIX environment variable name: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    format: Format,
    dir: &Path,
    size: &str,
    seed: u64,
    threads: &[u32],
    repeat: u32,
    thread_env: &str,
    tolerance: f64,
) -> anyhow::Result<()> {
    if repeat == 0 || repeat > 100 {
        return Err(Fixable::new(format!(
            "repeat count {repeat} is outside the supported range"
        ))
        .fix("choose a repeat count from 1 to 100")
        .into_error());
    }
    if !(tolerance > 0.0 && tolerance < 1.0) {
        return Err(Fixable::new(format!(
            "tolerance {tolerance} is outside the open interval (0, 1)"
        ))
        .fix("pass --tolerance as a fraction, e.g. 0.05 for 5%")
        .into_error());
    }

    let sol = Solution::load(dir)?;
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

    let host_cpus = crate::context::host_info().nproc;
    let sweep = if threads.is_empty() {
        default_threads(host_cpus)
    } else {
        threads.to_vec()
    };

    let pipeline = Pipeline::new(&sol)?;
    let mut policy = policy_for(&sol.problem);
    // A scaling sweep needs real cores. The default policy pins the run to CPU 0
    // and one CPU's worth of quota, which would force every thread count to the
    // same single core and report a flat curve that says nothing about scaling.
    policy.cpuset = None;
    policy.cpus = sweep.iter().copied().max().unwrap_or(1) as f64;

    let program = pipeline.build_solution()?;
    let input = pipeline.generate(size, seed)?;
    let expected = pipeline.reference_output(&input)?;
    let checker = if sol.problem.meta.judge == Judge::Checker {
        Some(pipeline.build_checker()?)
    } else {
        None
    };

    let mut warnings = Vec::new();
    if sweep.len() < 2 {
        warnings
            .push("a single thread count cannot show a curve; pass --threads 1,2,4".to_string());
    }
    if !sweep.contains(&1) {
        warnings.push(
            "no one-thread run in the sweep; speed-up is relative to the lowest thread count"
                .to_string(),
        );
    }
    if sweep.iter().copied().max().unwrap_or(1) > host_cpus {
        warnings.push(format!(
            "the sweep requests up to {} threads but the host reports {host_cpus} logical CPUs",
            sweep.iter().copied().max().unwrap_or(1)
        ));
    }

    let mut runs: Vec<ThreadSamples> = Vec::new();
    for &count in &sweep {
        let mut threaded = program.clone();
        with_thread_env(&mut threaded, thread_env, count)?;

        // Correctness gates the timing: a wrong parallel answer has no speed-up
        // worth reporting. Check the first run at each thread count, because a
        // solution can be correct serially and wrong in parallel.
        let probe = pipeline.run_program(&threaded, &input, &policy, false)?;
        let verdict = probe.verdict(policy.memory_bytes);
        if verdict != Verdict::Accepted {
            return Err(Fixable::new(format!(
                "the {count}-thread run did not complete cleanly: {verdict:?} (exit {}, signal {})",
                probe.exit_status, probe.signal
            ))
            .fix("make the solution correct at every thread count before measuring scaling")
            .into_error());
        }
        let matches = match &checker {
            Some(checker) => pipeline.check_output(
                checker,
                &input,
                &expected,
                probe.stdout.as_bytes(),
                &policy,
            )?,
            None => probe.stdout.as_bytes() == expected.as_slice(),
        };
        if !matches {
            return Err(
                Fixable::new(format!("the {count}-thread run produced a wrong answer"))
                    .fix(
                        "a wrong parallel answer has no speed-up to measure: fix correctness first",
                    )
                    .into_error(),
            );
        }

        let mut samples = Vec::with_capacity(repeat as usize);
        for _ in 0..repeat {
            let outcome = pipeline.run_program(&threaded, &input, &policy, false)?;
            if outcome.verdict(policy.memory_bytes) != Verdict::Accepted {
                return Err(
                    Fixable::new(format!("a {count}-thread run failed mid-sweep"))
                        .fix("a flaky solution cannot be measured for scaling")
                        .into_error(),
                );
            }
            samples.push(outcome.wall_ns);
        }
        let median_ns = median(&samples);
        runs.push(ThreadSamples {
            threads: count,
            samples_ns: samples,
            median_ns,
        });
    }

    let curve_input: Vec<(u32, Vec<u64>)> = runs
        .iter()
        .map(|run| (run.threads, run.samples_ns.clone()))
        .collect();
    let curve = scaling::curve(&curve_input, host_cpus, tolerance);

    let report = ScalingReport {
        problem: sol.problem.meta.id.clone(),
        language: format!("{:?}", sol.language).to_lowercase(),
        size: size.to_string(),
        seed,
        thread_env: thread_env.to_string(),
        kind: "indicative".to_string(),
        interpretation: "speed-up and efficiency are ratios of whole-process wall times; they move with the host, its load, and its core count".to_string(),
        curve,
        runs,
        warnings,
    };

    cmd::emit(
        format,
        || {
            println!(
                "scaling | {} [{}] size {size} seed {seed} (indicative)",
                report.problem, report.language
            );
            println!("  {}", report.interpretation);
            println!(
                "  thread env {thread_env}; host {} logical CPUs",
                report.curve.host_cpus
            );
            println!();
            println!(
                "  {:>8} {:>12} {:>10} {:>12}",
                "threads", "median", "speed-up", "efficiency"
            );
            for point in &report.curve.points {
                println!(
                    "  {:>8} {:>12} {:>10} {:>12}",
                    point.threads,
                    fmt_ns(point.median_ns),
                    point
                        .speedup
                        .map(|s| format!("{s:.3}x"))
                        .unwrap_or_else(|| "baseline".into()),
                    point
                        .efficiency
                        .map(|e| format!("{e:.3}"))
                        .unwrap_or_else(|| "-".into()),
                );
            }
            if let Some(saturation) = report.curve.saturation {
                println!();
                println!(
                    "  saturates at {saturation} threads (no further gain beyond {tolerance:.1}%)"
                );
            }
            for note in &report.curve.notes {
                println!("  note: {note}");
            }
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
        },
        &report,
    )?;

    Ok(())
}

/// The median of a set of samples, matching `scaling`'s integer convention so
/// the per-thread summary and the curve agree.
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
        (sorted[mid - 1] / 2) + (sorted[mid] / 2) + (sorted[mid - 1] % 2 + sorted[mid] % 2) / 2
    }
}

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
    use crate::pipeline::Program;
    use std::path::PathBuf;

    fn program(prefix: Vec<&str>) -> Program {
        Program {
            host_dir: PathBuf::from("/tmp"),
            guest_binary: "/prog/sol".to_string(),
            args: vec![],
            runtime_argv: None,
            runtime_prefix: vec![],
            prefix: prefix.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn default_sweep_is_powers_of_two_up_to_the_cpu_count() {
        assert_eq!(default_threads(1), vec![1]);
        assert_eq!(default_threads(4), vec![1, 2, 4]);
        assert_eq!(default_threads(8), vec![1, 2, 4, 8]);
        assert_eq!(default_threads(3), vec![1, 2]);
    }

    #[test]
    fn thread_specs_are_sorted_and_de_duplicated() {
        assert_eq!(parse_threads("4,1,2,2").unwrap(), vec![1, 2, 4]);
        assert!(parse_threads("0").is_err());
        assert!(parse_threads("abc").is_err());
        assert!(parse_threads("300").is_err());
        assert!(parse_threads("").is_err());
    }

    #[test]
    fn thread_env_is_prepended_when_there_is_no_env_prefix() {
        let mut prog = program(vec![]);
        with_thread_env(&mut prog, "OMP_NUM_THREADS", 4).unwrap();
        assert_eq!(prog.prefix, vec!["env", "OMP_NUM_THREADS=4"]);
    }

    #[test]
    fn thread_env_replaces_an_existing_assignment_in_an_env_prefix() {
        let mut prog = program(vec!["env", "CCX_RUN=1"]);
        with_thread_env(&mut prog, "CCX_RUN", 8).unwrap();
        assert_eq!(prog.prefix, vec!["env", "CCX_RUN=8"]);
    }

    #[test]
    fn thread_env_replaces_the_go_runtime_pin_where_it_lives() {
        // The Go profile pins GOMAXPROCS=1 in runtime_prefix, not prefix. The
        // sweep must override it there, or the inner env shadows every count.
        let mut prog = program(vec![]);
        prog.runtime_prefix = vec!["env", "GOMAXPROCS=1", "GODEBUG=randautoseed=0"]
            .into_iter()
            .map(String::from)
            .collect();
        with_thread_env(&mut prog, "GOMAXPROCS", 8).unwrap();
        assert_eq!(
            prog.runtime_prefix,
            vec!["env", "GOMAXPROCS=8", "GODEBUG=randautoseed=0"]
        );
        assert!(prog.prefix.is_empty());
    }

    #[test]
    fn thread_env_is_inserted_after_env_when_the_variable_is_absent() {
        let mut prog = program(vec!["env", "CCX_RUN=1"]);
        with_thread_env(&mut prog, "OMP_NUM_THREADS", 2).unwrap();
        assert_eq!(prog.prefix, vec!["env", "OMP_NUM_THREADS=2", "CCX_RUN=1"]);
    }

    #[test]
    fn median_matches_the_core_convention() {
        assert_eq!(median(&[5, 1, 3]), 3);
        assert_eq!(median(&[1, 2, 3, 4]), 2);
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn invalid_variable_names_are_rejected() {
        let mut prog = program(vec![]);
        assert!(with_thread_env(&mut prog, "1BAD", 2).is_err());
        assert!(with_thread_env(&mut prog, "BAD NAME", 2).is_err());
        assert!(with_thread_env(&mut prog, "", 2).is_err());
    }
}
