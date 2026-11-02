//! Statistical ranking: tie groups.
//!
//! The platform's honesty rule reaches ranking: two results whose intervals
//! overlap are a tie, and a tie shares a rank. Nothing here decides that one
//! solution is faster than another when the counters cannot separate them.
//!
//! Rank is competition rank ("1224"): every member of a tie group carries the
//! rank of the group's first member, and the next distinct value skips the
//! ranks the group consumed. Lower is better for the deterministic metrics the
//! engine ranks on, so entries are ordered ascending.

use serde::{Deserialize, Serialize};

/// One candidate in a ranking. `uncertainty` is the half-width of the
/// interval around `value` (for example the bootstrap 95% CI half-width). A
/// missing interval is a point estimate: it ties only on exact equality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankEntry {
    pub label: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<f64>,
    /// Breaks ties for display order only; it never splits a rank group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tie_break: Option<f64>,
}

/// A set of entries that share a rank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankGroup {
    /// Competition rank, 1-based. Tied entries share the first member's rank.
    pub rank: u32,
    /// Indices into the input slice, ordered by `tie_break` then label.
    pub members: Vec<usize>,
}

impl RankGroup {
    pub fn is_tie(&self) -> bool {
        self.members.len() > 1
    }
}

/// Whether two entries' intervals overlap, so they cannot be separated.
fn intervals_overlap(a: &RankEntry, b: &RankEntry) -> bool {
    let a_half = a.uncertainty.unwrap_or(0.0).abs();
    let b_half = b.uncertainty.unwrap_or(0.0).abs();
    a.value - a_half <= b.value + b_half && b.value - b_half <= a.value + a_half
}

/// Group entries into statistical tie groups and assign competition ranks.
///
/// Ties are transitive: if A overlaps B and B overlaps C, all three share a
/// group even when A and C do not overlap. That is the conservative choice —
/// it refuses to claim a separation the intervals do not support.
pub fn rank_groups(entries: &[RankEntry]) -> Vec<RankGroup> {
    let n = entries.len();
    if n == 0 {
        return Vec::new();
    }

    // Order by value ascending; break equal values by tie_break then label so
    // the grouping is deterministic and independent of input order.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        entries[i]
            .value
            .partial_cmp(&entries[j].value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| match (entries[i].tie_break, entries[j].tie_break) {
                (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| entries[i].label.cmp(&entries[j].label))
    });

    // Union-find over overlapping intervals.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for a in 0..n {
        for b in (a + 1)..n {
            if intervals_overlap(&entries[a], &entries[b]) {
                let ra = find(&mut parent, a);
                let rb = find(&mut parent, b);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    // Collect groups in sorted order, preserving the first-seen position.
    let mut groups: Vec<RankGroup> = Vec::new();
    let mut group_of: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for &idx in &order {
        let root = find(&mut parent, idx);
        match group_of.get(&root) {
            Some(&group_index) => groups[group_index].members.push(idx),
            None => {
                group_of.insert(root, groups.len());
                groups.push(RankGroup {
                    rank: 0,
                    members: vec![idx],
                });
            }
        }
    }

    // Sort members inside each group by the display tie-break, then rank them.
    let mut next_rank = 1u32;
    for group in &mut groups {
        group.members.sort_by(|&i, &j| {
            match (entries[i].tie_break, entries[j].tie_break) {
                (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| entries[i].label.cmp(&entries[j].label))
        });
        group.rank = next_rank;
        next_rank += group.members.len() as u32;
    }

    groups
}

/// One candidate on a multi-objective frontier. Every axis is lower-is-better.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParetoEntry {
    pub label: String,
    pub axes: std::collections::BTreeMap<String, f64>,
}

/// Which entries are on the Pareto frontier.
///
/// Entry `a` dominates entry `b` when `a` is at least as good on every axis
/// and strictly better on at least one. The frontier is the set of entries no
/// other entry dominates. Entries missing any requested axis cannot be fairly
/// compared, so they are returned as `incomplete` rather than silently
/// dropped or treated as infinitely bad.
pub struct ParetoFrontier {
    /// Labels on the frontier, in input order.
    pub frontier: Vec<String>,
    /// label -> the label that dominates it, for dominated entries.
    pub dominated_by: std::collections::BTreeMap<String, String>,
    /// Labels missing at least one requested axis.
    pub incomplete: Vec<String>,
}

pub fn pareto_frontier(entries: &[ParetoEntry], axes: &[String]) -> ParetoFrontier {
    let complete: Vec<&ParetoEntry> = entries
        .iter()
        .filter(|entry| axes.iter().all(|axis| entry.axes.contains_key(axis)))
        .collect();
    let incomplete: Vec<String> = entries
        .iter()
        .filter(|entry| !axes.iter().all(|axis| entry.axes.contains_key(axis)))
        .map(|entry| entry.label.clone())
        .collect();

    let mut dominated_by = std::collections::BTreeMap::new();
    for candidate in &complete {
        for other in &complete {
            if candidate.label == other.label {
                continue;
            }
            let at_least_as_good = axes
                .iter()
                .all(|axis| other.axes[axis] <= candidate.axes[axis]);
            let strictly_better = axes
                .iter()
                .any(|axis| other.axes[axis] < candidate.axes[axis]);
            if at_least_as_good && strictly_better {
                dominated_by.insert(candidate.label.clone(), other.label.clone());
                break;
            }
        }
    }

    let frontier = complete
        .iter()
        .filter(|entry| !dominated_by.contains_key(&entry.label))
        .map(|entry| entry.label.clone())
        .collect();

    ParetoFrontier {
        frontier,
        dominated_by,
        incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, value: f64, uncertainty: Option<f64>) -> RankEntry {
        RankEntry {
            label: label.to_string(),
            value,
            uncertainty,
            tie_break: None,
        }
    }

    #[test]
    fn disjoint_intervals_get_distinct_ranks() {
        let entries = vec![
            entry("a", 100.0, Some(1.0)),
            entry("b", 200.0, Some(1.0)),
            entry("c", 300.0, Some(1.0)),
        ];
        let groups = rank_groups(&entries);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].rank, 1);
        assert_eq!(groups[1].rank, 2);
        assert_eq!(groups[2].rank, 3);
        assert!(groups.iter().all(|g| !g.is_tie()));
    }

    #[test]
    fn overlapping_intervals_share_a_rank() {
        let entries = vec![
            entry("a", 100.0, Some(5.0)),
            entry("b", 103.0, Some(5.0)),
            entry("c", 200.0, Some(1.0)),
        ];
        let groups = rank_groups(&entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rank, 1);
        assert!(groups[0].is_tie());
        assert_eq!(groups[0].members.len(), 2);
        // The next distinct value skips the consumed ranks (competition rank).
        assert_eq!(groups[1].rank, 3);
    }

    #[test]
    fn ties_are_transitive_across_a_chain() {
        // a~b and b~c, but a and c do not overlap directly.
        let entries = vec![
            entry("a", 100.0, Some(2.0)),
            entry("b", 104.0, Some(2.0)),
            entry("c", 108.0, Some(2.0)),
        ];
        let groups = rank_groups(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 3);
        assert_eq!(groups[0].rank, 1);
    }

    #[test]
    fn point_estimates_tie_only_on_exact_equality() {
        let entries = vec![entry("a", 100.0, None), entry("b", 100.0, None)];
        let groups = rank_groups(&entries);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].is_tie());

        let distinct = vec![entry("a", 100.0, None), entry("b", 100.0001, None)];
        assert_eq!(rank_groups(&distinct).len(), 2);
    }

    #[test]
    fn zero_width_intervals_do_not_fabricate_ties() {
        let entries = vec![entry("a", 100.0, Some(0.0)), entry("b", 101.0, Some(0.0))];
        assert_eq!(rank_groups(&entries).len(), 2);
    }

    #[test]
    fn tie_break_orders_members_without_splitting_the_rank() {
        let entries = vec![
            RankEntry {
                label: "slow".into(),
                value: 100.0,
                uncertainty: Some(5.0),
                tie_break: Some(9.0),
            },
            RankEntry {
                label: "fast".into(),
                value: 102.0,
                uncertainty: Some(5.0),
                tie_break: Some(1.0),
            },
        ];
        let groups = rank_groups(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
        // Display order follows the tie-break; the shared rank is unchanged.
        assert_eq!(entries[groups[0].members[0]].label, "fast");
        assert_eq!(entries[groups[0].members[1]].label, "slow");
    }

    #[test]
    fn ranking_is_independent_of_input_order() {
        let forward = vec![
            entry("a", 100.0, Some(5.0)),
            entry("b", 103.0, Some(5.0)),
            entry("c", 200.0, Some(1.0)),
        ];
        let mut backward = forward.clone();
        backward.reverse();
        let g1 = rank_groups(&forward);
        let g2 = rank_groups(&backward);
        let labels = |groups: &[RankGroup], entries: &[RankEntry]| {
            groups
                .iter()
                .map(|g| {
                    let mut names: Vec<String> = g
                        .members
                        .iter()
                        .map(|&i| entries[i].label.clone())
                        .collect();
                    names.sort();
                    (g.rank, names)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(&g1, &forward), labels(&g2, &backward));
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(rank_groups(&[]).is_empty());
    }

    fn pareto_entry(label: &str, axes: &[(&str, f64)]) -> ParetoEntry {
        ParetoEntry {
            label: label.to_string(),
            axes: axes
                .iter()
                .map(|(name, value)| (name.to_string(), *value))
                .collect(),
        }
    }

    #[test]
    fn dominated_entry_is_not_on_the_frontier() {
        let entries = vec![
            pareto_entry("naive", &[("instructions", 500.0), ("memory", 100.0)]),
            pareto_entry("great", &[("instructions", 200.0), ("memory", 100.0)]),
        ];
        let axes = vec!["instructions".to_string(), "memory".to_string()];
        let result = pareto_frontier(&entries, &axes);
        assert_eq!(result.frontier, vec!["great".to_string()]);
        assert_eq!(
            result.dominated_by.get("naive").map(String::as_str),
            Some("great")
        );
    }

    #[test]
    fn trade_off_keeps_both_entries_on_the_frontier() {
        // "fast" wins instructions, "lean" wins memory; neither dominates.
        let entries = vec![
            pareto_entry("fast", &[("instructions", 100.0), ("memory", 500.0)]),
            pareto_entry("lean", &[("instructions", 300.0), ("memory", 100.0)]),
        ];
        let axes = vec!["instructions".to_string(), "memory".to_string()];
        let result = pareto_frontier(&entries, &axes);
        assert_eq!(
            result.frontier,
            vec!["fast".to_string(), "lean".to_string()]
        );
        assert!(result.dominated_by.is_empty());
    }

    #[test]
    fn equal_entries_do_not_dominate_each_other() {
        let entries = vec![
            pareto_entry("a", &[("instructions", 100.0)]),
            pareto_entry("b", &[("instructions", 100.0)]),
        ];
        let axes = vec!["instructions".to_string()];
        let result = pareto_frontier(&entries, &axes);
        assert_eq!(result.frontier.len(), 2);
        assert!(result.dominated_by.is_empty());
    }

    #[test]
    fn entry_missing_an_axis_is_incomplete_not_dominated() {
        let entries = vec![
            pareto_entry("full", &[("instructions", 100.0), ("memory", 100.0)]),
            pareto_entry("partial", &[("instructions", 50.0)]),
        ];
        let axes = vec!["instructions".to_string(), "memory".to_string()];
        let result = pareto_frontier(&entries, &axes);
        assert_eq!(result.frontier, vec!["full".to_string()]);
        assert_eq!(result.incomplete, vec!["partial".to_string()]);
        assert!(result.dominated_by.is_empty());
    }

    #[test]
    fn pareto_needs_a_strict_win_on_one_axis() {
        // Identical on every axis means neither dominates the other.
        let entries = vec![
            pareto_entry("a", &[("instructions", 100.0), ("memory", 200.0)]),
            pareto_entry("b", &[("instructions", 100.0), ("memory", 200.0)]),
        ];
        let axes = vec!["instructions".to_string(), "memory".to_string()];
        let result = pareto_frontier(&entries, &axes);
        assert_eq!(result.frontier.len(), 2);
    }
}
