//! `ccx principles` — print the trust contract: the five metric kinds, what
//! each one promises, and the measurement principles the engine enforces.
//!
//! Part 1 of the work order is a list of non-negotiable principles. Rather than
//! leave them in a document that drifts, this command renders the same
//! `ccx_core::trust` data the tests assert, so a reviewer, the website, or CI
//! can read exactly what the engine guarantees and confirm the invariants hold.
//!
//! `--check` turns it into a gate: it exits non-zero when any invariant is
//! violated, so it can run in CI as a mechanical proof of the trust contract.

use crate::cmd;
use crate::Format;
use ccx_core::trust;

pub fn run(format: Format, check: bool) -> anyhow::Result<()> {
    let contract = trust::contract();
    let violations = trust::check_invariants();

    if check && !violations.is_empty() {
        for violation in &violations {
            eprintln!("trust contract violated: {violation}");
        }
        anyhow::bail!(
            "the trust contract is violated in {} place(s); see stderr",
            violations.len()
        );
    }

    cmd::emit(
        format,
        || {
            println!(
                "measurement trust contract (schema v{})",
                contract.schema_version
            );
            println!();
            println!("metric kinds");
            for spec in &contract.kinds {
                let rank = if spec.rankable {
                    "rankable"
                } else {
                    "never ranked"
                };
                println!("  {:<11} {}", spec.kind.to_string(), spec.definition);
                println!(
                    "              deterministic={} host_dependent={} {}",
                    spec.deterministic, spec.host_dependent, rank
                );
            }
            println!("  {:<11} no value was produced", "unavailable");
            println!("              shown as: unavailable with the reason, never 0");
            println!();
            println!("metric methods (provenance)");
            println!("  {}", contract.methods.join(", "));
            println!();
            println!("principles");
            for principle in &contract.principles {
                println!("  {}", principle.statement);
                println!("      enforced by: {}", principle.enforcement);
            }
            println!();
            println!(
                "invariants: {}",
                if contract.invariants_hold {
                    "hold".to_string()
                } else {
                    format!("VIOLATED ({} violation(s))", violations.len())
                }
            );
        },
        &contract,
    )
}

#[cfg(test)]
mod tests {
    use ccx_core::capsule::MetricKind;
    use ccx_core::trust;

    #[test]
    fn contract_lists_the_five_kinds_with_unavailable_not_a_variant() {
        let contract = trust::contract();
        // Four serialisable kinds plus the `unavailable` outcome.
        assert_eq!(contract.kinds.len(), 4);
        assert!(contract
            .kinds
            .iter()
            .any(|spec| spec.kind == MetricKind::Predicted));
        assert!(contract.invariants_hold);
    }

    #[test]
    fn predicted_is_never_rankable() {
        // The mechanical statement of principle 2: a static prediction must not
        // be able to enter a ranking, and the contract says so.
        let predicted = trust::spec(MetricKind::Predicted);
        assert!(!predicted.rankable);
        assert!(!predicted.host_dependent);
        assert!(predicted.deterministic);
    }

    #[test]
    fn the_provenance_vocabulary_includes_the_new_methods() {
        let contract = trust::contract();
        assert!(contract.methods.contains(&"llvm_mca"));
        assert!(contract.methods.contains(&"cgroup"));
        assert!(contract.methods.contains(&"perf"));
    }

    #[test]
    fn every_kind_says_absence_is_unavailable_never_zero() {
        for spec in trust::KIND_SPECS {
            assert!(
                spec.absence_shown_as.starts_with("unavailable"),
                "kind {} must say its absence is unavailable, got {:?}",
                spec.kind,
                spec.absence_shown_as
            );
            assert!(
                spec.absence_shown_as.contains("never 0"),
                "kind {} must say absence is never 0, got {:?}",
                spec.kind,
                spec.absence_shown_as
            );
        }
    }
}
