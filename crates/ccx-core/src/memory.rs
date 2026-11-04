//! Memory profile: the heap high-water mark and where the live bytes sit.
//!
//! Peak RSS (`getrusage`) answers "how much memory did the process touch"; it
//! is honest but *indicative*, because it counts pages the allocator and the
//! loader reserved, not bytes the program asked for. This module adds the
//! complementary measurement the capsule could not express: the peak live
//! *heap* bytes, measured by Valgrind Massif's fixed allocator instrumentation,
//! and the allocation sites that hold those bytes at the peak.
//!
//! Massif records the live heap after each allocation, so the maximum
//! `mem_heap_B` across snapshots is the true high-water mark of live heap
//! bytes for the traced process. It is a real measurement of the program's own
//! request stream, so it carries the `measured` label, not `simulated`.
//!
//! The size-class histogram groups the peak snapshot's leaves into power-of-two
//! buckets. Massif groups its tree by allocation *site*, so each bucket is
//! "peak live bytes held by sites whose block is this size", not a count of
//! individual allocations. The note in [`MassifProfile::notes`] says so.

use serde::{Deserialize, Serialize};

/// One leaf of the peak snapshot's allocation tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocSite {
    /// Live bytes held at this site in the peak snapshot.
    pub bytes: u64,
    /// Resolved location text, e.g. `main (sol.c:12)`, or `(unknown)`.
    pub location: String,
}

/// One power-of-two size bucket of the peak snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeClass {
    /// Inclusive lower bound, a power of two.
    pub lower_bytes: u64,
    /// Exclusive upper bound; `None` for the top, open-ended bucket.
    pub upper_bytes: Option<u64>,
    /// Live bytes in this bucket at the peak.
    pub bytes: u64,
    /// Number of sites whose leaf falls in this bucket.
    pub sites: u64,
}

/// The parsed result of one Massif run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MassifProfile {
    /// Maximum `mem_heap_B` over every snapshot: the live-heap high-water mark.
    pub peak_live_bytes: u64,
    /// Index of the snapshot that reached the high-water mark.
    pub peak_snapshot: u64,
    /// How many snapshots Massif recorded.
    pub snapshot_count: u64,
    /// Allocation sites holding the peak live bytes, largest first.
    pub sites: Vec<AllocSite>,
    /// Peak live bytes grouped into power-of-two size classes, ascending.
    pub size_classes: Vec<SizeClass>,
    /// Honesty notes about what the numbers do and do not mean.
    pub notes: Vec<String>,
}

/// Parse a Massif output file.
///
/// Returns `None` only when the file is not a Massif profile at all (no
/// `mem_heap_B` line). An empty profile — a program that never allocated — is a
/// real result and yields a zero high-water mark with no sites.
pub fn parse_massif(text: &str) -> Option<MassifProfile> {
    let mut peak_live_bytes = 0u64;
    let mut peak_snapshot = 0u64;
    let mut snapshot_count = 0u64;
    let mut current_snapshot = 0u64;
    let mut saw_heap = false;

    // Nodes of the peak snapshot's tree, in the order Massif printed them.
    let mut peak_nodes: Vec<(u32, u64, String)> = Vec::new();
    let mut in_peak_tree = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("snapshot=") {
            snapshot_count += 1;
            current_snapshot = rest.trim().parse().unwrap_or(0);
            continue;
        }
        if let Some(rest) = line.strip_prefix("mem_heap_B=") {
            saw_heap = true;
            let current_heap = rest.trim().parse().unwrap_or(0);
            if current_heap > peak_live_bytes {
                peak_live_bytes = current_heap;
                peak_snapshot = current_snapshot;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("heap_tree=") {
            in_peak_tree = rest.trim() == "peak";
            continue;
        }
        if in_peak_tree {
            if let Some(node) = parse_massif_node(line) {
                peak_nodes.push(node);
            }
        }
    }

    if !saw_heap {
        return None;
    }

    let sites = peak_leaves(&peak_nodes);
    let size_classes = size_classes(&sites);

    let mut notes = vec![
        "peak live heap bytes is the maximum mem_heap_B over Massif snapshots: a \
         measurement of the program's own allocation stream, not a page count"
            .to_string(),
        "the size classes group Massif's per-site leaves into power-of-two \
         buckets, so each bucket is peak live bytes held by sites of that block \
         size, not a count of individual allocations"
            .to_string(),
    ];
    if sites.is_empty() && peak_live_bytes > 0 {
        notes.push(
            "the peak snapshot carried no per-site leaves; the total is exact but \
             the attribution is unavailable for this build"
                .to_string(),
        );
    }
    if sites.iter().all(|site| site.location == "(unknown)") && !sites.is_empty() {
        notes.push(
            "no allocation site resolved to a source location; build with debug \
             info to attribute the live bytes"
                .to_string(),
        );
    }

    Some(MassifProfile {
        peak_live_bytes,
        peak_snapshot,
        snapshot_count,
        sites,
        size_classes,
        notes,
    })
}

/// Parse one Massif tree line: `<indent>n<k>: <bytes> <rest>`.
fn parse_massif_node(line: &str) -> Option<(u32, u64, String)> {
    let depth = line.len() - line.trim_start_matches(' ').len();
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('n')?;
    let (kind, rest) = rest.split_once(':')?;
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim_start();
    let (bytes_text, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let bytes: u64 = bytes_text.trim().parse().ok()?;
    let location = rest.trim();
    let location = if location.is_empty() {
        "(unknown)".to_string()
    } else {
        location.to_string()
    };
    Some((depth as u32, bytes, location))
}

/// Keep only the leaves of the tree: nodes with no deeper node after them.
fn peak_leaves(nodes: &[(u32, u64, String)]) -> Vec<AllocSite> {
    let mut sites: Vec<AllocSite> = Vec::new();
    for (index, (depth, bytes, location)) in nodes.iter().enumerate() {
        let is_leaf = nodes
            .get(index + 1)
            .map(|(next_depth, _, _)| next_depth <= depth)
            .unwrap_or(true);
        if is_leaf && *bytes > 0 {
            sites.push(AllocSite {
                bytes: *bytes,
                location: location.clone(),
            });
        }
    }
    sites.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| a.location.cmp(&b.location))
    });
    sites
}

/// Group leaf bytes into power-of-two size classes, ascending.
fn size_classes(sites: &[AllocSite]) -> Vec<SizeClass> {
    let mut buckets: std::collections::BTreeMap<u32, (u64, u64)> =
        std::collections::BTreeMap::new();
    for site in sites {
        if site.bytes == 0 {
            continue;
        }
        let exponent = 63 - site.bytes.leading_zeros();
        let entry = buckets.entry(exponent).or_insert((0, 0));
        entry.0 += site.bytes;
        entry.1 += 1;
    }
    buckets
        .into_iter()
        .map(|(exponent, (bytes, count))| SizeClass {
            lower_bytes: 1u64 << exponent,
            upper_bytes: (exponent < 63).then(|| 1u64 << (exponent + 1)),
            bytes,
            sites: count,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
desc: --massif-out-file=/work/m.out
cmd: /prog/sol
time_unit: i
#-----------
snapshot=0
#-----------
time=0
mem_heap_B=0
mem_heap_extra_B=0
mem_stacks_B=0
heap_tree=empty
#-----------
snapshot=1
#-----------
time=100
mem_heap_B=1048576
mem_heap_extra_B=8
mem_stacks_B=0
heap_tree=empty
#-----------
snapshot=2
#-----------
time=200
mem_heap_B=3145728
mem_heap_extra_B=16
mem_stacks_B=0
heap_tree=peak
n2: 3145728 (heap allocation functions) malloc/new/new[], --alloc-fns, etc.
 n0: 2097152 0x109178: main (sol.c:5)
 n0: 1048576 0x10916A: main (sol.c:4)
#-----------
snapshot=3
#-----------
time=300
mem_heap_B=2097152
mem_heap_extra_B=16
mem_stacks_B=0
heap_tree=empty
";

    #[test]
    fn peak_is_the_maximum_heap_over_snapshots() {
        let p = parse_massif(SAMPLE).unwrap();
        assert_eq!(p.peak_live_bytes, 3_145_728);
        assert_eq!(p.peak_snapshot, 2);
        assert_eq!(p.snapshot_count, 4);
    }

    #[test]
    fn leaves_are_sites_largest_first() {
        let p = parse_massif(SAMPLE).unwrap();
        assert_eq!(p.sites.len(), 2);
        assert_eq!(p.sites[0].bytes, 2_097_152);
        assert_eq!(p.sites[0].location, "0x109178: main (sol.c:5)");
        assert_eq!(p.sites[1].bytes, 1_048_576);
    }

    #[test]
    fn size_classes_are_power_of_two_buckets() {
        let p = parse_massif(SAMPLE).unwrap();
        // 1048576 -> 2^20, 2097152 -> 2^21
        assert_eq!(p.size_classes.len(), 2);
        assert_eq!(p.size_classes[0].lower_bytes, 1 << 20);
        assert_eq!(p.size_classes[0].bytes, 1_048_576);
        assert_eq!(p.size_classes[1].lower_bytes, 1 << 21);
        assert_eq!(p.size_classes[1].bytes, 2_097_152);
    }

    #[test]
    fn nested_leaves_are_not_reported_as_sites() {
        // A parent with two children: only the children are leaves.
        let text = "\
snapshot=0
mem_heap_B=300
heap_tree=peak
n2: 300 (root)
 n1: 200 (branch)
  n0: 150 0x1: f (a.c:1)
  n0: 50 0x2: g (a.c:2)
 n0: 100 0x3: h (a.c:3)
";
        let p = parse_massif(text).unwrap();
        let total: u64 = p.sites.iter().map(|s| s.bytes).sum();
        assert_eq!(total, 300);
        assert_eq!(p.sites.len(), 3);
    }

    #[test]
    fn a_program_that_never_allocated_is_a_zero_profile_not_an_error() {
        let text = "\
snapshot=0
mem_heap_B=0
heap_tree=empty
";
        let p = parse_massif(text).unwrap();
        assert_eq!(p.peak_live_bytes, 0);
        assert!(p.sites.is_empty());
    }

    #[test]
    fn non_massif_input_is_rejected() {
        assert!(parse_massif("nothing to see here\n").is_none());
    }
}
