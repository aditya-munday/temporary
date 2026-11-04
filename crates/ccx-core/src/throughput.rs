//! Static throughput: what a scheduling model predicts the instruction stream
//! can sustain, without running it.
//!
//! Every other number in this engine comes from an execution: Cachegrind
//! simulates the instruction stream, `perf_event_open` reads the hardware, the
//! wall-clock engine times the process. This module adds the one number that
//! none of them can produce for a solution the host cannot run: the *predicted*
//! throughput of a basic block under a fixed machine model.
//!
//! The source is `llvm-mca`, LLVM's machine-code analyzer. It is fed the same
//! assembly the compiler emitted for the measured build, and it reports the
//! cycle count, IPC, and per-resource pressure its model assigns to that block.
//! Because the block is analyzed statically, the result is a property of the
//! code and the named CPU model, not of any host that ran it — which is exactly
//! why it must be labelled `predicted` and never used as a ranking signal.
//!
//! Two honesty properties are enforced here and in the command layer:
//!
//! * the CPU model is named in the report, because the prediction is only
//!   meaningful relative to it, and
//! * instructions the model does not understand (calls, returns, anything with
//!   unmodeled side effects) are listed, because llvm-mca substitutes fixed
//!   penalties for them and the totals inherit that guess.

use serde::{Deserialize, Serialize};

/// One scheduling resource and the pressure the block puts on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourcePressure {
    /// Resource name as the model reports it, e.g. `SKLPort0`.
    pub resource: String,
    /// Sum of per-instruction usage across the block, in cycles. This is a
    /// model quantity: it says how busy the model thinks the resource is, not
    /// how busy the hardware was.
    pub cycles: f64,
    /// `cycles` divided by the number of static instructions in the block:
    /// the model's average occupancy per instruction.
    pub per_instruction: f64,
}

/// The parsed result of one `llvm-mca` analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McaReport {
    /// CPU model the prediction is relative to, e.g. `skylake`.
    pub cpu: String,
    /// Target triple the model assumed, e.g. `x86_64-pc-linux-gnu`.
    pub triple: String,
    /// Iterations the analyzer ran its model for.
    pub iterations: u64,
    /// Static instructions in the analyzed block.
    pub instruction_count: usize,
    /// Total modelled instructions (static count times iterations).
    pub instructions: u64,
    /// Modelled cycles to retire the block.
    pub total_cycles: f64,
    /// Modelled micro-ops.
    pub total_uops: u64,
    /// Modelled instructions per cycle.
    pub ipc: f64,
    /// Modelled micro-ops per cycle.
    pub uops_per_cycle: f64,
    /// The model's dispatch width.
    pub dispatch_width: f64,
    /// The model's reciprocal-throughput estimate for the block: the number of
    /// cycles one iteration of the block costs when it is not latency-bound.
    pub block_rthroughput: f64,
    /// Resources under pressure, busiest first.
    pub pressure: Vec<ResourcePressure>,
    /// Instructions the model does not understand, so their cost is a
    /// substitute rather than a prediction.
    pub unmodeled: Vec<String>,
    /// Honesty notes about what the numbers mean.
    pub notes: Vec<String>,
}

#[derive(Deserialize)]
struct RawReport {
    #[serde(rename = "CodeRegions", default)]
    code_regions: Vec<RawRegion>,
    #[serde(rename = "SimulationParameters", default)]
    simulation_parameters: Option<RawSimulation>,
    #[serde(rename = "TargetInfo", default)]
    target_info: Option<RawTarget>,
}

#[derive(Deserialize)]
struct RawSimulation {
    #[serde(rename = "-mcpu", default)]
    mcpu: Option<String>,
    #[serde(rename = "-mtriple", default)]
    mtriple: Option<String>,
}

#[derive(Deserialize)]
struct RawTarget {
    #[serde(rename = "CPUName", default)]
    cpu_name: Option<String>,
    #[serde(rename = "Resources", default)]
    resources: Vec<String>,
}

#[derive(Deserialize)]
struct RawRegion {
    #[serde(rename = "SummaryView", default)]
    summary: Option<RawSummary>,
    #[serde(rename = "Instructions", default)]
    instructions: Vec<String>,
    #[serde(rename = "ResourcePressureView", default)]
    resource_pressure: Option<RawResourcePressure>,
}

#[derive(Deserialize)]
struct RawSummary {
    #[serde(rename = "BlockRThroughput", default)]
    block_rthroughput: f64,
    #[serde(rename = "DispatchWidth", default)]
    dispatch_width: f64,
    #[serde(rename = "IPC", default)]
    ipc: f64,
    #[serde(rename = "Instructions", default)]
    instructions: u64,
    #[serde(rename = "Iterations", default)]
    iterations: u64,
    #[serde(rename = "TotalCycles", default)]
    total_cycles: f64,
    #[serde(rename = "TotaluOps", default)]
    total_uops: u64,
    #[serde(rename = "uOpsPerCycle", default)]
    uops_per_cycle: f64,
}

#[derive(Deserialize)]
struct RawResourcePressure {
    #[serde(rename = "ResourcePressureInfo", default)]
    info: Vec<RawPressureEntry>,
}

#[derive(Deserialize)]
struct RawPressureEntry {
    #[serde(rename = "ResourceIndex", default)]
    resource_index: usize,
    #[serde(rename = "ResourceUsage", default)]
    usage: f64,
}

/// Make an llvm-mca resource name safe to print.
///
/// When a processor exposes several sub-resources that share a prefix,
/// llvm-mca names them `Prefix.<index>` but writes the index as a raw byte
/// rather than its digits, so the JSON carries control characters such as
/// `Zn4LSU.\u{2}`. Those bytes are the sub-resource index, so rendering each
/// control character as its decimal value recovers the intended
/// `Zn4LSU.2` while keeping distinct resources distinct.
fn sanitize_resource_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_control() {
            sanitized.push_str(&(character as u32).to_string());
        } else {
            sanitized.push(character);
        }
    }
    sanitized
}

/// Instructions whose modelled cost is a substitute rather than a prediction.
///
/// llvm-mca charges a fixed penalty for a call and ignores program-counter
/// updates around a return. Reporting them is what keeps the totals honest.
fn unmodeled_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mnemonic = lower.split_whitespace().next().unwrap_or("");
    matches!(mnemonic, "call" | "callq" | "ret" | "retq")
        || mnemonic.starts_with("call")
        || mnemonic.starts_with("ret")
}

/// Parse `llvm-mca -json` output into a [`McaReport`].
///
/// Returns `None` when the input is not a usable analysis: no code region, no
/// summary, or no instructions. A report with a zero instruction count is a
/// real answer about an empty block, but it carries no prediction, so it is
/// treated as unusable here rather than reported as a zero.
pub fn parse_mca_json(text: &str) -> Option<McaReport> {
    let raw: RawReport = serde_json::from_str(text).ok()?;
    let region = raw.code_regions.into_iter().next()?;
    let summary = region.summary?;
    if region.instructions.is_empty() {
        return None;
    }
    let instruction_count = region.instructions.len();

    let resources = raw
        .target_info
        .as_ref()
        .map(|target| target.resources.clone())
        .unwrap_or_default();
    let cpu = raw
        .target_info
        .as_ref()
        .and_then(|target| target.cpu_name.clone())
        .or_else(|| {
            raw.simulation_parameters
                .as_ref()
                .and_then(|params| params.mcpu.clone())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let triple = raw
        .simulation_parameters
        .as_ref()
        .and_then(|params| params.mtriple.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Aggregate per-resource usage across the block. The JSON carries one
    // entry per (instruction, resource) pair; the sum over a resource is the
    // model's total occupancy of that resource for one pass of the block.
    let mut totals = vec![0.0f64; resources.len()];
    if let Some(pressure) = region.resource_pressure {
        for entry in pressure.info {
            if entry.resource_index < totals.len() {
                totals[entry.resource_index] += entry.usage;
            }
        }
    }
    let mut sanitized_names = false;
    let mut pressure: Vec<ResourcePressure> = resources
        .into_iter()
        .enumerate()
        .map(|(index, resource)| {
            let name = sanitize_resource_name(&resource);
            sanitized_names |= name != resource;
            ResourcePressure {
                resource: name,
                cycles: totals[index],
                per_instruction: totals[index] / instruction_count as f64,
            }
        })
        .filter(|entry| entry.cycles > 0.0)
        .collect();
    pressure.sort_by(|a, b| {
        b.cycles
            .partial_cmp(&a.cycles)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut unmodeled: Vec<String> = Vec::new();
    for instruction in &region.instructions {
        if unmodeled_instruction(instruction) {
            let text = instruction.trim().to_string();
            if !unmodeled.contains(&text) {
                unmodeled.push(text);
            }
        }
    }

    let mut notes = vec![
        "predicted by llvm-mca's machine model for the named CPU; not a measurement \
         of any host"
            .to_string(),
    ];
    if !unmodeled.is_empty() {
        notes.push(format!(
            "{} instruction(s) are unmodeled (calls, returns); llvm-mca substitutes a \
             fixed penalty, so the cycle and IPC totals inherit that guess",
            unmodeled.len()
        ));
    }
    if summary.instructions == 0 {
        notes.push(
            "the model retired no instructions for this block; treat the totals as empty"
                .to_string(),
        );
    }
    if sanitized_names {
        notes.push(
            "some resource names contained control characters from llvm-mca's sub-resource \
             indices; they were rendered as their decimal value, e.g. `Zn4LSU.2`"
                .to_string(),
        );
    }

    Some(McaReport {
        cpu,
        triple,
        iterations: summary.iterations,
        instruction_count,
        instructions: summary.instructions,
        total_cycles: summary.total_cycles,
        total_uops: summary.total_uops,
        ipc: summary.ipc,
        uops_per_cycle: summary.uops_per_cycle,
        dispatch_width: summary.dispatch_width,
        block_rthroughput: summary.block_rthroughput,
        pressure,
        unmodeled,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `llvm-mca-19 -json` capture for a three-instruction block
    /// (`addq; movq; retq`) under `-mcpu=skylake`, trimmed to the fields the
    /// parser reads. Real output keeps the parser honest about the exact
    /// spelling of keys and the shape of the pressure view.
    const SKYLAKE_ADD: &str = r#"{
      "CodeRegions": [
        {
          "Instructions": [
            "addq\t%rsi, %rdi",
            "movq\t%rdi, %rax",
            "retq"
          ],
          "ResourcePressureView": {
            "ResourcePressureInfo": [
              {"InstructionIndex": 0, "ResourceIndex": 2, "ResourceUsage": 0.01},
              {"InstructionIndex": 0, "ResourceIndex": 3, "ResourceUsage": 0.98},
              {"InstructionIndex": 1, "ResourceIndex": 7, "ResourceUsage": 1.0},
              {"InstructionIndex": 2, "ResourceIndex": 2, "ResourceUsage": 0.98},
              {"InstructionIndex": 2, "ResourceIndex": 4, "ResourceUsage": 0.5}
            ]
          },
          "SummaryView": {
            "BlockRThroughput": 1.0,
            "DispatchWidth": 6,
            "IPC": 2.727272727272727,
            "Instructions": 300,
            "Iterations": 100,
            "TotalCycles": 110,
            "TotaluOps": 500,
            "uOpsPerCycle": 4.545454545454546
          }
        }
      ],
      "SimulationParameters": {
        "-march": "x86_64",
        "-mcpu": "skylake",
        "-mtriple": "x86_64-pc-linux-gnu"
      },
      "TargetInfo": {
        "CPUName": "skylake",
        "Resources": [
          "SKLDivider", "SKLFPDivider", "SKLPort0", "SKLPort1", "SKLPort2",
          "SKLPort3", "SKLPort4", "SKLPort5", "SKLPort6", "SKLPort7"
        ]
      }
    }"#;

    #[test]
    fn summary_fields_are_read_verbatim() {
        let report = parse_mca_json(SKYLAKE_ADD).expect("fixture parses");
        assert_eq!(report.cpu, "skylake");
        assert_eq!(report.triple, "x86_64-pc-linux-gnu");
        assert_eq!(report.iterations, 100);
        assert_eq!(report.instructions, 300);
        assert_eq!(report.instruction_count, 3);
        assert_eq!(report.total_cycles, 110.0);
        assert_eq!(report.total_uops, 500);
        assert_eq!(report.dispatch_width, 6.0);
        assert_eq!(report.block_rthroughput, 1.0);
        assert!((report.ipc - 2.727272727272727).abs() < 1e-12);
    }

    #[test]
    fn pressure_is_aggregated_per_resource_and_busiest_first() {
        let report = parse_mca_json(SKYLAKE_ADD).expect("fixture parses");
        // SKLPort0 (resource index 2) is used by two instructions: 0.01 + 0.98.
        let port0 = report
            .pressure
            .iter()
            .find(|entry| entry.resource == "SKLPort0")
            .expect("port0 present");
        assert!((port0.cycles - 0.99).abs() < 1e-9);
        assert!((port0.per_instruction - 0.33).abs() < 1e-9);
        // Busiest first.
        for pair in report.pressure.windows(2) {
            assert!(pair[0].cycles >= pair[1].cycles);
        }
    }

    #[test]
    fn calls_and_returns_are_reported_as_unmodeled() {
        let report = parse_mca_json(SKYLAKE_ADD).expect("fixture parses");
        assert_eq!(report.unmodeled, vec!["retq".to_string()]);
        assert!(report.notes.iter().any(|note| note.contains("unmodeled")));
    }

    #[test]
    fn control_characters_in_resource_names_are_rendered_as_indices() {
        // llvm-mca writes sub-resource indices as raw bytes, so the JSON can
        // contain `Zn4LSU.\u{2}` where it means `Zn4LSU.2`.
        let fixture = r#"{"CodeRegions":[{"Instructions":["nop"],
            "ResourcePressureView":{"ResourcePressureInfo":[
                {"InstructionIndex":0,"ResourceIndex":0,"ResourceUsage":1.0},
                {"InstructionIndex":0,"ResourceIndex":1,"ResourceUsage":2.0}
            ]},
            "SummaryView":{"Instructions":100,"Iterations":100,"IPC":1.0}}],
            "TargetInfo":{"CPUName":"znver4","Resources":["Zn4LSU.\u0002","Zn4LSU.\u0001"]}}"#;
        let report = parse_mca_json(fixture).expect("fixture parses");
        let names: Vec<&str> = report
            .pressure
            .iter()
            .map(|entry| entry.resource.as_str())
            .collect();
        assert!(names.contains(&"Zn4LSU.2"), "got {names:?}");
        assert!(names.contains(&"Zn4LSU.1"), "got {names:?}");
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("control characters")));
    }

    #[test]
    fn a_report_without_instructions_is_rejected() {
        let empty = r#"{"CodeRegions":[{"Instructions":[],
            "SummaryView":{"Instructions":0,"Iterations":100}}]}"#;
        assert!(parse_mca_json(empty).is_none());
    }

    #[test]
    fn non_json_input_is_rejected() {
        assert!(parse_mca_json("warning: found a call\nIterations: 100\n").is_none());
    }

    #[test]
    fn missing_target_info_still_yields_a_report() {
        let minimal = r#"{"CodeRegions":[{"Instructions":["nop"],
            "SummaryView":{"Instructions":100,"Iterations":100,"IPC":1.0}}]}"#;
        let report = parse_mca_json(minimal).expect("minimal report parses");
        assert_eq!(report.cpu, "unknown");
        assert_eq!(report.triple, "unknown");
        assert!(report.pressure.is_empty());
    }
}
