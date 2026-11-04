//! `ccx throughput` — the static throughput a machine model predicts for the
//! solution's assembly.
//!
//! This is the one number in the engine that no execution produces. `llvm-mca`
//! reads the compiler's own output for the measured build and reports the
//! cycles, IPC, and resource pressure its model assigns to that block. Because
//! nothing ran, the result is labelled `predicted` and is never a ranking
//! signal: it describes the code relative to a named CPU model, not the host.
//!
//! The report always names the model and lists the instructions the model does
//! not understand, because those are the two things that make a prediction
//! trustworthy or not.

use crate::cmd::{self, Fixable};
use crate::context::Solution;
use crate::pipeline::Pipeline;
use crate::Format;
use ccx_core::throughput::McaReport;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ThroughputReport {
    pub toolchain: String,
    /// Always `predicted`: this is a model, not a measurement.
    pub kind: String,
    /// Whether a report was produced at all.
    pub available: bool,
    /// Why the report is unavailable, when it is.
    pub unavailable_reason: Option<String>,
    pub report: Option<McaReport>,
    pub warnings: Vec<String>,
}

pub fn run(format: Format, dir: &Path, mcpu: Option<&str>, iterations: u64) -> anyhow::Result<()> {
    if iterations == 0 {
        return Err(Fixable::new("--iterations must be at least 1".to_string())
            .fix("the analyzer needs at least one iteration to model a cycle count")
            .into_error());
    }
    let sol = Solution::load(dir)?;
    let pipeline = Pipeline::new(&sol)?;
    let tc = sol.toolchain()?;
    sol.ensure_dirs()?;

    let mut warnings = Vec::new();
    let analysis = pipeline.static_throughput(mcpu, iterations)?;

    let Some(report) = analysis.report else {
        let reason = "the pinned Clang image does not contain llvm-mca, so no static \
                      throughput model is available on this host"
            .to_string();
        let report = ThroughputReport {
            toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
            kind: "predicted".to_string(),
            available: false,
            unavailable_reason: Some(reason),
            report: None,
            warnings,
        };
        return cmd::emit(
            format,
            || {
                println!("static throughput ({}, size from build)", report.toolchain);
                println!(
                    "  unavailable: {}",
                    report
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("no analyzer in this image")
                );
                println!("  note: this prediction would be labelled `predicted`, never `measured`");
            },
            &report,
        );
    };

    // Multi-file solutions compile to one assembly file per source; the
    // analyzer sees the first. Say so rather than implying the whole program
    // was modeled.
    if analysis.assembly_files > 1 {
        warnings.push(format!(
            "this solution compiles to {} assembly files; only the first was analyzed, \
             so the prediction covers one translation unit",
            analysis.assembly_files
        ));
    }

    let report = ThroughputReport {
        toolchain: format!("{} {}", tc.name.profile_id(), tc.version),
        kind: "predicted".to_string(),
        available: true,
        unavailable_reason: None,
        report: Some(report),
        warnings,
    };

    cmd::emit(
        format,
        || {
            let mca = report.report.as_ref().expect("report is present");
            println!(
                "static throughput ({}, kind={})",
                report.toolchain, report.kind
            );
            println!("  model:            {} ({})", mca.cpu, mca.triple);
            println!("  iterations:       {}", mca.iterations);
            println!("  static instrs:    {}", mca.instruction_count);
            println!("  total cycles:     {:.1}  (modelled)", mca.total_cycles);
            println!("  IPC:              {:.2}", mca.ipc);
            println!("  uOps/cycle:       {:.2}", mca.uops_per_cycle);
            println!("  dispatch width:   {:.0}", mca.dispatch_width);
            println!(
                "  block rthroughput:{:.1}  (cycles per iteration when not latency-bound)",
                mca.block_rthroughput
            );
            if !mca.pressure.is_empty() {
                println!("\nresource pressure (busiest first, cycles per block pass)");
                println!("{:<16} {:>12} {:>16}", "resource", "cycles", "per-instr");
                for entry in mca.pressure.iter().take(12) {
                    println!(
                        "{:<16} {:>12.2} {:>16.3}",
                        entry.resource, entry.cycles, entry.per_instruction
                    );
                }
            }
            if !mca.unmodeled.is_empty() {
                println!("\nunmodeled instructions (fixed penalty substituted)");
                for instruction in &mca.unmodeled {
                    println!("  {instruction}");
                }
            }
            for note in &mca.notes {
                println!("\nnote: {note}");
            }
            for warning in &report.warnings {
                println!("\nwarning: {warning}");
            }
        },
        &report,
    )
}
