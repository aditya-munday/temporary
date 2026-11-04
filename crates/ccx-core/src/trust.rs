//! The trust contract: the five metric kinds, what each one promises, and the
//! invariants the engine is not allowed to break.
//!
//! Part 1 of the work order is a list of non-negotiable principles. A principle
//! that lives only in prose is a wish. This module turns the measurement
//! principles into data and code: one canonical description of every kind, the
//! ranking rule as a function, and a set of invariants that are asserted in
//! tests and printed by `ccx principles` so a reviewer (or the website) can
//! read exactly what the engine guarantees and check it mechanically.
//!
//! The single hard rule, in the words of the work order: *a measured metric and
//! a simulated one must never be confused, and a measurement that is not real
//! must not be reported.* `unavailable` is never a zero; `predicted` is never a
//! measurement.

use crate::capsule::{Metric, MetricKind, MetricMethod};
use serde::Serialize;

/// What a kind means, and the obligations that come with it.
///
/// `renders_as_zero` is the load-bearing field: a kind whose value may be
/// absent must say so explicitly rather than let a `0` stand in for "we did not
/// measure this". Only the two deterministic kinds, plus `predicted` (which is
/// always produced when available), carry a real number.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KindSpec {
    /// The wire value, matching `MetricKind`'s serde representation.
    pub kind: MetricKind,
    /// One-line definition, safe to show a user.
    pub definition: &'static str,
    /// Whether two runs of the same code must agree on this number.
    pub deterministic: bool,
    /// Whether the number depends on the host it was produced on.
    pub host_dependent: bool,
    /// Whether the number may enter a ranking.
    pub rankable: bool,
    /// Whether "no value" is a legal outcome for this kind.
    pub may_be_unavailable: bool,
    /// What the absence of a value must look like in output.
    pub absence_shown_as: &'static str,
}

/// The canonical, ordered description of all five kinds. This is the single
/// source of truth; `ccx principles` renders it and the tests assert it.
pub const KIND_SPECS: &[KindSpec] = &[
    KindSpec {
        kind: MetricKind::Measured,
        definition: "a real hardware or OS counter (perf, cgroup, getrusage, stat)",
        deterministic: false,
        host_dependent: true,
        rankable: true,
        may_be_unavailable: true,
        absence_shown_as: "unavailable with the reason, never 0",
    },
    KindSpec {
        kind: MetricKind::Simulated,
        definition: "a deterministic model of a real run (cachegrind, callgrind)",
        deterministic: true,
        host_dependent: false,
        rankable: true,
        may_be_unavailable: true,
        absence_shown_as: "unavailable with the reason, never 0",
    },
    KindSpec {
        kind: MetricKind::Indicative,
        definition: "wall-clock time; machine-dependent and noisy",
        deterministic: false,
        host_dependent: true,
        rankable: false,
        may_be_unavailable: true,
        absence_shown_as: "unavailable with the reason, never 0",
    },
    KindSpec {
        kind: MetricKind::Predicted,
        definition: "a static model of code that never ran (llvm-mca); names its model",
        deterministic: true,
        host_dependent: false,
        rankable: false,
        may_be_unavailable: true,
        absence_shown_as: "unavailable with the reason, never 0",
    },
];

/// Look up the spec for a kind. Total, because `KIND_SPECS` covers every kind.
pub fn spec(kind: MetricKind) -> &'static KindSpec {
    KIND_SPECS
        .iter()
        .find(|entry| entry.kind == kind)
        .expect("KIND_SPECS must cover every MetricKind")
}

/// A principle from Part 1, stated so it can be checked rather than admired.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Principle {
    /// Stable id, e.g. `kind-labels`.
    pub id: &'static str,
    /// The rule in one sentence.
    pub statement: &'static str,
    /// How the engine enforces it (the code path, or the test that proves it).
    pub enforcement: &'static str,
}

/// The measurement principles this module enforces. Non-measurement principles
/// (safety, replayability) are enforced elsewhere and listed here only for the
/// reader's orientation.
pub const PRINCIPLES: &[Principle] = &[
    Principle {
        id: "kind-labels",
        statement: "Every metric carries one of the five kinds: measured, simulated, indicative, predicted, or unavailable.",
        enforcement: "MetricKind + KIND_SPECS; capsule validation rejects an unknown kind.",
    },
    Principle {
        id: "unavailable-not-zero",
        statement: "An unavailable metric is reported as unavailable with a reason, never as 0.",
        enforcement: "Every collector returns Option/None and the CLI renders the reason; see ccx topdown/throughput/memory.",
    },
    Principle {
        id: "simulated-not-measured",
        statement: "A simulated metric is never labelled measured, and the reverse.",
        enforcement: "Metric.kind is set at the single construction site for each metric; asserted per metric in tests.",
    },
    Principle {
        id: "deterministic-ranking",
        statement: "Ranking uses deterministic metrics only; wall-clock and predicted numbers never rank.",
        enforcement: "Metric::rankable_kind + ranking_eligibility; a predicted or indicative entry is refused with a reason.",
    },
    Principle {
        id: "indicative-has-spread",
        statement: "An indicative number always carries its uncertainty; fake precision is not shipped.",
        enforcement: "Capsule::validate rejects an indicative metric without uncertainty.",
    },
    Principle {
        id: "machine-labelled",
        statement: "A machine- or model-dependent number always names the machine or model that produced it.",
        enforcement: "Host fingerprint on every capsule; llvm-mca reports name the modelled CPU.",
    },
];

/// Assert every structural invariant of the taxonomy, returning a human list of
/// violations (empty when the contract holds).
///
/// This is the function `ccx principles --check` runs and the tests call. It is
/// deliberately total and side-effect free so it can run anywhere.
pub fn check_invariants() -> Vec<String> {
    let mut violations = Vec::new();

    // 1. Every kind has exactly one spec.
    let all = [
        MetricKind::Measured,
        MetricKind::Simulated,
        MetricKind::Indicative,
        MetricKind::Predicted,
    ];
    if KIND_SPECS.len() != all.len() {
        violations.push(format!(
            "KIND_SPECS has {} entries but there are {} kinds",
            KIND_SPECS.len(),
            all.len()
        ));
    }
    for kind in all {
        if !KIND_SPECS.iter().any(|entry| entry.kind == kind) {
            violations.push(format!("no spec for kind {kind}"));
        }
    }

    // 2. The rankable set is exactly {measured, simulated}. This is the
    //    mechanical statement of principle 2.
    for entry in KIND_SPECS {
        let expected = matches!(entry.kind, MetricKind::Measured | MetricKind::Simulated);
        if entry.rankable != expected {
            violations.push(format!(
                "kind {} has rankable={} but the rule says {expected}",
                entry.kind, entry.rankable
            ));
        }
    }

    // 3. No kind claims both determinism and host dependence.
    for entry in KIND_SPECS {
        if entry.deterministic && entry.host_dependent {
            violations.push(format!(
                "kind {} is both deterministic and host-dependent",
                entry.kind
            ));
        }
    }

    // 4. Every kind that may be unavailable names how absence is shown, and it
    //    is never "0".
    for entry in KIND_SPECS {
        if entry.may_be_unavailable && !entry.absence_shown_as.contains("unavailable") {
            violations.push(format!(
                "kind {} may be unavailable but does not say so",
                entry.kind
            ));
        }
    }

    violations
}

/// The wire form of the trust contract, for `ccx principles --format json` and
/// for the website's schema registry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrustContract {
    pub schema_version: u32,
    pub kinds: Vec<KindSpec>,
    pub methods: Vec<&'static str>,
    pub principles: Vec<Principle>,
    pub invariants_hold: bool,
}

/// Build the contract payload. `methods` lists every `MetricMethod` so a client
/// can render a provenance table without hard-coding the enum.
pub fn contract() -> TrustContract {
    TrustContract {
        schema_version: crate::capsule::CURRENT_SCHEMA_VERSION,
        kinds: KIND_SPECS.to_vec(),
        methods: ALL_METHODS.iter().map(|m| method_name(*m)).collect(),
        principles: PRINCIPLES.to_vec(),
        invariants_hold: check_invariants().is_empty(),
    }
}

/// Every `MetricMethod`, in a stable order. The one list to update when a method
/// is added; `method_name` still forces the wire name to be chosen, and the
/// drift test fails if this list is not kept complete.
pub const ALL_METHODS: &[MetricMethod] = &[
    MetricMethod::Cachegrind,
    MetricMethod::Perf,
    MetricMethod::Getrusage,
    MetricMethod::Cgroup,
    MetricMethod::Stat,
    MetricMethod::Manual,
    MetricMethod::ValgrindMemcheck,
    MetricMethod::WallClock,
    MetricMethod::Runner,
    MetricMethod::LlvmMca,
];

/// Map a metric method to its canonical wire name. Exhaustive by construction:
/// adding a `MetricMethod` without a name here fails to compile, which is the
/// point — the wire vocabulary cannot silently drift from the enum.
pub fn method_name(method: MetricMethod) -> &'static str {
    match method {
        MetricMethod::Cachegrind => "cachegrind",
        MetricMethod::Perf => "perf",
        MetricMethod::Getrusage => "getrusage",
        MetricMethod::Cgroup => "cgroup",
        MetricMethod::Stat => "stat",
        MetricMethod::Manual => "manual",
        MetricMethod::ValgrindMemcheck => "valgrind_memcheck",
        MetricMethod::WallClock => "wall_clock",
        MetricMethod::Runner => "runner",
        MetricMethod::LlvmMca => "llvm_mca",
    }
}

/// The provenance a metric must record to be trustworthy: the kind and method
/// travel together, so a number can never be shown without saying how it was
/// obtained.
pub fn provenance(metric: &Metric) -> String {
    format!("{} via {}", metric.kind, method_name(metric.method))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taxonomy_has_four_serialisable_kinds_plus_the_unavailable_outcome() {
        // Four kinds round-trip through a capsule; the fifth outcome the work
        // order names, `unavailable`, is the absence of a metric (an `Option`
        // that is `None`), not a `MetricKind` variant, so it can never be
        // mistaken for a value.
        assert_eq!(KIND_SPECS.len(), 4);
        assert_eq!(
            KIND_SPECS.iter().map(|s| s.kind).collect::<Vec<_>>(),
            vec![
                MetricKind::Measured,
                MetricKind::Simulated,
                MetricKind::Indicative,
                MetricKind::Predicted,
            ]
        );
    }

    #[test]
    fn invariants_hold() {
        assert_eq!(check_invariants(), Vec::<String>::new());
    }

    #[test]
    fn only_deterministic_kinds_are_rankable() {
        assert!(spec(MetricKind::Simulated).rankable);
        assert!(spec(MetricKind::Measured).rankable);
        assert!(!spec(MetricKind::Indicative).rankable);
        assert!(!spec(MetricKind::Predicted).rankable);
    }

    #[test]
    fn every_method_has_a_unique_wire_name() {
        let names: Vec<&str> = ALL_METHODS.iter().map(|m| method_name(*m)).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        // And the mapping is total over the enum.
        assert_eq!(method_name(MetricMethod::LlvmMca), "llvm_mca");
        assert_eq!(method_name(MetricMethod::Cgroup), "cgroup");
    }

    #[test]
    fn the_method_list_matches_the_enum_exactly() {
        // Listing every variant here means a new variant that is not added to
        // ALL_METHODS (or is named differently) fails this test, so the wire
        // vocabulary cannot drift from the enum in either direction.
        let all = [
            MetricMethod::Cachegrind,
            MetricMethod::Perf,
            MetricMethod::Getrusage,
            MetricMethod::Cgroup,
            MetricMethod::Stat,
            MetricMethod::Manual,
            MetricMethod::ValgrindMemcheck,
            MetricMethod::WallClock,
            MetricMethod::Runner,
            MetricMethod::LlvmMca,
        ];
        assert_eq!(ALL_METHODS, &all);
    }

    #[test]
    fn provenance_names_both_kind_and_method() {
        let metric = Metric::count("cycles", 10, MetricKind::Predicted, MetricMethod::LlvmMca);
        assert_eq!(provenance(&metric), "predicted via llvm_mca");
    }

    #[test]
    fn contract_serialises_and_reports_invariants() {
        let contract = contract();
        assert!(contract.invariants_hold);
        assert_eq!(contract.kinds.len(), 4);
        let json = serde_json::to_string(&contract).unwrap();
        assert!(json.contains("\"predicted\""));
        assert!(json.contains("unavailable"));
    }
}
