//! Hotspot attribution: which functions and source lines spend the
//! instructions.
//!
//! The counter says *what* changed; attribution says *where*. Two views are
//! produced from the same Callgrind run, so they always agree:
//!
//! * a flat table of functions and of source lines, parsed from
//!   `callgrind_annotate --auto=yes` output, and
//! * a call-graph folded-stack export (and optional flame-graph SVG) built
//!   from the Callgrind file itself, which keeps the caller/callee structure
//!   that the flat table discards.
//!
//! Instruction counts here are Cachegrind's fixed model, so they carry the
//! `simulated` label, exactly like the `instructions` metric they decompose.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One row of `callgrind_annotate`'s function table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionHotspot {
    pub file: String,
    pub function: String,
    /// Shared object the function was resolved in, when the row names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub ir: u64,
    pub ir_pct: f64,
}

/// One annotated source line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineHotspot {
    /// 1-based source line number. `callgrind_annotate --auto=yes` prints the
    /// whole file in order, so the row's position is its line number.
    pub line: u64,
    pub ir: u64,
    pub ir_pct: f64,
    pub text: String,
}

/// Every annotated line of one source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceHotspot {
    pub source: String,
    pub lines: Vec<LineHotspot>,
}

/// The parsed result of one annotation pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotspotReport {
    /// Event columns the annotation carried, e.g. `["Ir"]`.
    pub events: Vec<String>,
    pub program_total: u64,
    /// Functions, most expensive first.
    pub functions: Vec<FunctionHotspot>,
    pub sources: Vec<SourceHotspot>,
    /// Honesty notes: anything the parser had to interpret, and the label.
    pub notes: Vec<String>,
}

fn split_count_and_rest(line: &str) -> Option<(u64, &str)> {
    let start = line.find(|c: char| !c.is_whitespace())?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let token = &rest[..end];
    let value = token.replace(',', "").parse::<u64>().ok()?;
    Some((value, &rest[end..]))
}

fn strip_percent(rest: &str) -> (f64, &str) {
    let trimmed = rest.trim_start();
    if let Some(after_open) = trimmed.strip_prefix('(') {
        if let Some(close) = after_open.find(')') {
            let pct = after_open[..close]
                .trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f64>()
                .unwrap_or(0.0);
            return (pct, &after_open[close + 1..]);
        }
    }
    (0.0, trimmed)
}

fn is_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-')
}

fn parse_function_row(line: &str) -> Option<FunctionHotspot> {
    let (ir, rest) = split_count_and_rest(line)?;
    let (ir_pct, rest) = strip_percent(rest);
    let name_field = rest.trim();
    if name_field.is_empty() || name_field.ends_with("file:function") {
        return None;
    }
    let (name, object) = match (name_field.rfind(" ["), name_field.ends_with(']')) {
        (Some(idx), true) => (
            name_field[..idx].to_string(),
            Some(name_field[idx + 2..name_field.len() - 1].to_string()),
        ),
        _ => (name_field.to_string(), None),
    };
    // `file:function`. The split is at the first colon: a Linux path cannot
    // contain one, but a demangled C++ function name can (and often does).
    let (file, function) = match name.split_once(':') {
        Some((file, function)) => (file.to_string(), function.to_string()),
        None => (String::new(), name.clone()),
    };
    Some(FunctionHotspot {
        file,
        function,
        object,
        ir,
        ir_pct,
    })
}

/// Parse one annotated source row. Returns the instruction count and text.
/// A `.` count means the line did not execute.
fn parse_annotated_row(line: &str) -> Option<(u64, f64, String)> {
    let start = line.find(|c: char| !c.is_whitespace())?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let token = &rest[..end];
    let ir = if token == "." {
        0
    } else {
        token.replace(',', "").parse::<u64>().ok()?
    };
    let (ir_pct, rest) = strip_percent(&rest[end..]);
    Some((ir, ir_pct, rest.trim_start().to_string()))
}

/// Parse the text produced by `callgrind_annotate --auto=yes`.
pub fn parse_annotate(text: &str) -> HotspotReport {
    let lines: Vec<&str> = text.lines().collect();
    let mut events = Vec::new();
    for line in &lines {
        if let Some(rest) = line.trim().strip_prefix("Events shown:") {
            events = rest.split_whitespace().map(str::to_string).collect();
            if !events.is_empty() {
                break;
            }
        }
    }
    if events.is_empty() {
        for line in &lines {
            if let Some(rest) = line.trim().strip_prefix("Events recorded:") {
                events = rest.split_whitespace().map(str::to_string).collect();
                if !events.is_empty() {
                    break;
                }
            }
        }
    }

    let mut program_total = 0u64;
    let mut functions = Vec::new();
    let mut sources = Vec::new();
    let mut notes = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.ends_with("PROGRAM TOTALS") {
            if let Some((value, _)) = split_count_and_rest(trimmed) {
                program_total = value;
            }
            i += 1;
            continue;
        }
        if trimmed.ends_with("file:function") {
            i += 1;
            // A rule separates the header from the rows.
            while i < lines.len() && (is_separator(lines[i]) || lines[i].trim().is_empty()) {
                i += 1;
            }
            while i < lines.len() && !is_separator(lines[i]) {
                if let Some(row) = parse_function_row(lines[i]) {
                    functions.push(row);
                }
                i += 1;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("-- Auto-annotated source:") {
            let source = rest.trim().to_string();
            i += 1;
            // The section opens with a rule, then the `Ir` column header.
            while i < lines.len()
                && (is_separator(lines[i]) || lines[i].trim().is_empty() || lines[i].trim() == "Ir")
            {
                i += 1;
            }
            let mut rows = Vec::new();
            let mut line_number = 1u64;
            while i < lines.len() && !is_separator(lines[i]) {
                if let Some((ir, ir_pct, text)) = parse_annotated_row(lines[i]) {
                    rows.push(LineHotspot {
                        line: line_number,
                        ir,
                        ir_pct,
                        text,
                    });
                    line_number += 1;
                }
                i += 1;
            }
            if rows.is_empty() {
                notes.push(format!(
                    "{source}: no annotated lines; the source may not be reachable at its recorded path"
                ));
            }
            sources.push(SourceHotspot {
                source,
                lines: rows,
            });
            continue;
        }
        i += 1;
    }

    functions.sort_by(|a, b| b.ir.cmp(&a.ir).then_with(|| a.function.cmp(&b.function)));
    notes.push(
        "instruction attribution is Cachegrind's fixed model (simulated), not a hardware sample"
            .to_string(),
    );

    HotspotReport {
        events,
        program_total,
        functions,
        sources,
        notes,
    }
}

/// One call edge in the Callgrind call graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    pub callee: String,
    /// Inclusive instruction cost attributed to this call.
    pub cost: u64,
}

/// A function-level call graph, built from the Callgrind output file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallGraph {
    /// Self (exclusive) instruction cost per function.
    pub self_cost: BTreeMap<String, u64>,
    /// Outgoing calls per function, in first-seen order.
    pub edges: BTreeMap<String, Vec<CallEdge>>,
}

/// Callgrind compresses repeated names: `fn=(12) main` defines id 12, and a
/// later `cfn=(12)` refers back to it. Resolve a `key=...` value against the
/// table, defining it when a name is present.
fn resolve_name(raw: &str, names: &mut BTreeMap<String, String>) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix('(') {
        if let Some(close) = rest.find(')') {
            let id = rest[..close].to_string();
            let name = rest[close + 1..].trim().to_string();
            if name.is_empty() {
                return names.get(&id).cloned().unwrap_or_default();
            }
            names.insert(id, name.clone());
            return name;
        }
    }
    trimmed.to_string()
}

fn parse_cost_line(line: &str) -> Option<u64> {
    let first = line.chars().next()?;
    if !(first.is_ascii_digit() || first == '*' || first == '+') {
        return None;
    }
    // A cost row is `position cost [cost...]`. The first cost column is the
    // primary event (Ir); any further columns are the other events Callgrind
    // recorded for cache and branch simulation.
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}

/// Parse the Callgrind output file into a call graph.
pub fn parse_callgrind(text: &str) -> CallGraph {
    let mut graph = CallGraph::default();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut callee: Option<String> = None;
    let mut in_call = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix("fn=") {
            let name = resolve_name(value, &mut names);
            current = Some(name.clone());
            graph.self_cost.entry(name).or_insert(0);
            callee = None;
            in_call = false;
            continue;
        }
        if let Some(value) = line.strip_prefix("cfn=") {
            callee = Some(resolve_name(value, &mut names));
            in_call = false;
            continue;
        }
        if line.starts_with("calls=") {
            in_call = true;
            continue;
        }
        if line.starts_with("fl=")
            || line.starts_with("fi=")
            || line.starts_with("cfi=")
            || line.starts_with("ob=")
            || line.starts_with("cob=")
            || line.contains(": ")
        {
            continue;
        }
        if let Some(cost) = parse_cost_line(line) {
            if let Some(current) = &current {
                if in_call {
                    if let Some(callee) = &callee {
                        let edges = graph.edges.entry(current.clone()).or_default();
                        if let Some(existing) = edges.iter_mut().find(|e| &e.callee == callee) {
                            existing.cost += cost;
                        } else {
                            edges.push(CallEdge {
                                callee: callee.clone(),
                                cost,
                            });
                        }
                    }
                    in_call = false;
                } else {
                    *graph.self_cost.entry(current.clone()).or_insert(0) += cost;
                }
            }
        }
    }
    graph
}

impl CallGraph {
    /// Flatten the graph into folded stacks, one per leaf frame.
    ///
    /// Each entry is `frame;frame;... self_cost`. A flame graph draws a parent
    /// frame as the sum of its children, so the leaf's exclusive cost is the
    /// right weight. Recursion is broken at the point it repeats so a cycle
    /// cannot loop forever.
    pub fn folded_stacks(&self) -> Vec<(String, u64)> {
        let mut is_callee: BTreeSet<&str> = BTreeSet::new();
        for edges in self.edges.values() {
            for edge in edges {
                is_callee.insert(edge.callee.as_str());
            }
        }
        // Root at `main` when it exists. The process entry points (the dynamic
        // loader, `_start`) otherwise dominate a short run and bury the code
        // the author actually wrote.
        let mut roots: Vec<String> = if self.self_cost.contains_key("main") {
            vec!["main".to_string()]
        } else {
            self.self_cost
                .keys()
                .filter(|name| !is_callee.contains(name.as_str()))
                .cloned()
                .collect()
        };
        if roots.is_empty() {
            roots = self.self_cost.keys().cloned().collect();
        }
        roots.sort();

        let mut out = Vec::new();
        for root in roots {
            let mut path = BTreeSet::new();
            path.insert(root.clone());
            self.walk(&root, &root, &mut path, &mut out);
        }
        out
    }

    fn walk(
        &self,
        node: &str,
        stack: &str,
        path: &mut BTreeSet<String>,
        out: &mut Vec<(String, u64)>,
    ) {
        let self_cost = self.self_cost.get(node).copied().unwrap_or(0);
        if self_cost > 0 {
            out.push((stack.to_string(), self_cost));
        }
        if let Some(edges) = self.edges.get(node) {
            for edge in edges {
                let child_stack = format!("{stack};{}", edge.callee);
                if path.contains(&edge.callee) {
                    // A recursion or a shared callee re-entered: keep the edge
                    // as a leaf so its cost is not lost, without recursing.
                    out.push((child_stack, edge.cost));
                    continue;
                }
                path.insert(edge.callee.clone());
                self.walk(&edge.callee, &child_stack, path, out);
                path.remove(&edge.callee);
            }
        }
    }
}

struct FlameNode {
    self_cost: u64,
    children: BTreeMap<String, FlameNode>,
}

impl FlameNode {
    fn new() -> Self {
        FlameNode {
            self_cost: 0,
            children: BTreeMap::new(),
        }
    }

    fn total(&self) -> u64 {
        self.self_cost + self.children.values().map(FlameNode::total).sum::<u64>()
    }

    fn insert(&mut self, frames: &[&str], cost: u64) {
        if frames.is_empty() {
            self.self_cost += cost;
            return;
        }
        let child = self
            .children
            .entry(frames[0].to_string())
            .or_insert_with(FlameNode::new);
        child.insert(&frames[1..], cost);
    }
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render folded stacks as a self-contained flame-graph SVG.
///
/// The picture is a drawing of Cachegrind's fixed model, so it is labelled
/// `simulated` in the title rather than presented as a hardware profile.
pub fn render_flame_svg(folded: &[(String, u64)], title: &str) -> String {
    const WIDTH: f64 = 1200.0;
    const ROW: f64 = 18.0;
    const FONT: f64 = 11.0;

    let mut root = FlameNode::new();
    for (stack, cost) in folded {
        let frames: Vec<&str> = stack.split(';').collect();
        root.insert(&frames, *cost);
    }
    let mut rects = String::new();
    let mut depth_max = 0usize;
    layout(&root, 0.0, WIDTH, 0, &mut rects, &mut depth_max);
    let height = (depth_max as f64 + 1.0) * ROW + 24.0;

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{height:.0}\" \
viewBox=\"0 0 {WIDTH} {height:.0}\" font-family=\"monospace\">\n\
<rect width=\"{WIDTH}\" height=\"{height:.0}\" fill=\"#ffffff\"/>\n\
<text x=\"8\" y=\"16\" font-size=\"{FONT}\" fill=\"#111111\">{}</text>\n\
{rects}</svg>\n",
        escape_xml(title)
    )
}

fn layout(
    node: &FlameNode,
    x: f64,
    width: f64,
    depth: usize,
    rects: &mut String,
    depth_max: &mut usize,
) {
    const ROW: f64 = 18.0;
    let mut cursor = x;
    let parent_total = node.total().max(1) as f64;
    for (name, child) in &node.children {
        let child_width = width * (child.total() as f64 / parent_total);
        if child_width < 0.5 {
            continue;
        }
        *depth_max = (*depth_max).max(depth + 1);
        let shade = 40 + ((depth * 40) % 160) as u32;
        let fill = format!(
            "#{:02x}{:02x}{:02x}",
            255 - shade,
            200 - shade / 2,
            120 + shade / 2
        );
        rects.push_str(&format!(
            "<rect x=\"{cursor:.1}\" y=\"{:.1}\" width=\"{child_width:.1}\" height=\"{:.1}\" \
fill=\"{fill}\" stroke=\"#ffffff\"/>\n",
            24.0 + depth as f64 * ROW,
            ROW - 1.0
        ));
        if child_width > 40.0 {
            let label = if name.len() > 40 { &name[..40] } else { name };
            rects.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"#111111\">{}</text>\n",
                cursor + 3.0,
                24.0 + depth as f64 * ROW + 12.0,
                escape_xml(label)
            ));
        }
        layout(child, cursor, child_width, depth + 1, rects, depth_max);
        cursor += child_width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANNOTATE: &str = "\
--------------------------------------------------------------------------------
Profile data file '/work/scratch/cg.out' (creator: callgrind-3.24.0)
--------------------------------------------------------------------------------
Events recorded: Ir
Events shown:     Ir
Auto-annotation:  on

--------------------------------------------------------------------------------
Ir
--------------------------------------------------------------------------------
275,115,917 (100.0%)  PROGRAM TOTALS

--------------------------------------------------------------------------------
Ir                    file:function
--------------------------------------------------------------------------------
275,000,017 (99.96%)  cgtest3.c:main [/work/scratch/sol]
    2,784 ( 0.00%)  ./stdio-common/printf.c:printf [/usr/lib/libc.so.6]

--------------------------------------------------------------------------------
-- Auto-annotated source: cgtest3.c
--------------------------------------------------------------------------------
Ir

          .           #include <stdio.h>
          .
          1 ( 0.00%)      long s = 0;
125,000,004 (45.44%)      for (long i = 0; i < n; i++) {
150,000,000 (54.52%)          if (i & 1) s += i; else s -= i;
      2,784 ( 0.00%)  => ./stdio-common/printf.c:printf (1x)
         3 ( 0.00%)  }

--------------------------------------------------------------------------------
Ir
--------------------------------------------------------------------------------
275,000,017 (99.96%)  events annotated
";

    #[test]
    fn annotate_parses_totals_functions_and_lines() {
        let report = parse_annotate(ANNOTATE);
        assert_eq!(report.events, vec!["Ir"]);
        assert_eq!(report.program_total, 275_115_917);
        assert_eq!(report.functions.len(), 2);
        assert_eq!(report.functions[0].function, "main");
        assert_eq!(report.functions[0].file, "cgtest3.c");
        assert_eq!(report.functions[0].ir, 275_000_017);
        assert_eq!(
            report.functions[0].object.as_deref(),
            Some("/work/scratch/sol")
        );
        assert_eq!(report.functions[1].function, "printf");

        assert_eq!(report.sources.len(), 1);
        let source = &report.sources[0];
        assert_eq!(source.source, "cgtest3.c");
        // Line numbers come from the row's position: the whole file is printed.
        assert_eq!(source.lines[0].line, 1);
        assert!(source.lines[0].text.contains("#include"));
        assert_eq!(source.lines[0].ir, 0);
        assert_eq!(source.lines[3].line, 4);
        assert_eq!(source.lines[3].ir, 125_000_004);
        assert!(source.lines[3].text.contains("for (long i"));
        // A call row is kept, marked by its `=>` text.
        assert!(source.lines.iter().any(|line| line.text.starts_with("=>")));
        assert!(report.notes.iter().any(|note| note.contains("simulated")));
    }

    const CALLGRIND: &str = "\
# callgrind format
version: 1
creator: callgrind-3.24.0
positions: line
events: Ir
summary: 1000

ob=(1) /work/scratch/sol
fl=(1) /src/main.c
fn=(1) main
1 10
cfn=(2) work
calls=1 5
2 900
3 5

fl=(1) /src/main.c
fn=(2) work
4 800
cfn=(3) helper
calls=2 5
5 100

fn=(3) helper
6 100
";

    #[test]
    fn callgrind_parses_self_costs_and_edges() {
        let graph = parse_callgrind(CALLGRIND);
        assert_eq!(graph.self_cost["main"], 15);
        assert_eq!(graph.self_cost["work"], 800);
        assert_eq!(graph.self_cost["helper"], 100);
        let main_edges = &graph.edges["main"];
        assert_eq!(main_edges.len(), 1);
        assert_eq!(main_edges[0].callee, "work");
        assert_eq!(main_edges[0].cost, 900);
    }

    #[test]
    fn folded_stacks_root_at_main_and_break_cycles() {
        let graph = parse_callgrind(CALLGRIND);
        let folded = graph.folded_stacks();
        // main's self cost, then work's, then helper's, each with its stack.
        assert!(folded.contains(&("main".to_string(), 15)));
        assert!(folded.contains(&("main;work".to_string(), 800)));
        assert!(folded.contains(&("main;work;helper".to_string(), 100)));
    }

    #[test]
    fn recursion_terminates_and_keeps_cost() {
        let cyclic = "\
fn=(1) main
1 5
cfn=(2) recur
calls=1 1
2 50

fn=(2) recur
3 10
cfn=(2) recur
calls=1 1
4 40
";
        let graph = parse_callgrind(cyclic);
        let folded = graph.folded_stacks();
        assert!(!folded.is_empty());
        assert!(folded.iter().any(|(stack, _)| stack.starts_with("main")));
    }

    #[test]
    fn flame_svg_is_well_formed_and_has_frames() {
        let folded = vec![
            ("main".to_string(), 15),
            ("main;work".to_string(), 800),
            ("main;work;helper".to_string(), 100),
        ];
        let svg = render_flame_svg(&folded, "hotspots (simulated)");
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.matches("<rect").count() >= 4); // background + 3 frames
        assert!(svg.contains("work"));
        assert!(svg.contains("simulated"));
    }

    #[test]
    fn empty_annotation_is_handled() {
        let report = parse_annotate("");
        assert_eq!(report.program_total, 0);
        assert!(report.functions.is_empty());
        assert!(report.sources.is_empty());
    }
}
