//! Capsule parser robustness (Group H1).
//!
//! Three properties, all deterministic so they can run in CI without a
//! fuzzing harness:
//!
//! 1. Every committed arena capsule loads and validates for its language, so
//!    the parser keeps a golden for each supported language rather than only
//!    the C++ fixtures.
//! 2. `Capsule::from_json` never panics, whatever the input. It either returns
//!    a validated capsule or an error, and a hostile document must not take the
//!    process down.
//! 3. Migration reaches the current schema from every historical version, and
//!    an unparseable or future version is rejected rather than silently
//!    accepted.

use ccx_core::capsule::{migrate, Capsule, Language, CURRENT_SCHEMA_VERSION};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<repo>/crates/ccx-core`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// `(arena directory, expected language)`. One entry per ranked language, so a
/// language cannot regress without a fixture failing.
const LANGUAGE_GOLDENS: &[(&str, Language)] = &[
    ("arena/c-multifile", Language::C),
    ("arena/good", Language::Cpp),
    ("arena/rust", Language::Rust),
    ("arena/d", Language::D),
    ("arena/zig", Language::Zig),
    ("arena/odin", Language::Odin),
    ("arena/go", Language::Go),
    ("arena/asm", Language::AsmX86_64),
];

fn newest_capsule(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("capsule") && name.ends_with(".json")
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

#[test]
fn every_language_has_a_loadable_validating_golden() {
    let root = repo_root();
    for (directory, expected) in LANGUAGE_GOLDENS {
        let dir = root.join(directory);
        let capsule_path = newest_capsule(&dir)
            .unwrap_or_else(|| panic!("no capsule fixture in {}", dir.display()));
        let text = std::fs::read_to_string(&capsule_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", capsule_path.display()));
        let capsule = Capsule::from_json(&text)
            .unwrap_or_else(|e| panic!("{} must load: {e}", capsule_path.display()));
        assert_eq!(
            capsule.source.language,
            *expected,
            "{} records the wrong language",
            capsule_path.display()
        );
        assert_eq!(
            capsule.schema_version,
            CURRENT_SCHEMA_VERSION,
            "{} must migrate to the current schema",
            capsule_path.display()
        );
        // Round-trip: the canonical serialisation must reload to an equal
        // capsule, so a golden stays a golden after a schema bump.
        let encoded = capsule.to_json_pretty().unwrap();
        let decoded = Capsule::from_json(&encoded).unwrap();
        assert_eq!(
            decoded,
            capsule,
            "{} did not round-trip",
            capsule_path.display()
        );
    }
}

/// A tiny deterministic PRNG so the "fuzz" corpus is identical on every run
/// and on every machine. Reproducibility applies to the tests too.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn valid_fixture() -> String {
    let root = repo_root();
    let path = root.join("crates/ccx-core/tests/fixtures/capsule-v15.json");
    std::fs::read_to_string(path).expect("capsule-v15 fixture")
}

#[test]
fn parser_never_panics_on_mutated_documents() {
    let base: Value = serde_json::from_str(&valid_fixture()).unwrap();
    let keys: Vec<String> = base.as_object().unwrap().keys().cloned().collect();
    let mut rng = XorShift(0x9E3779B97F4A7C15);

    for iteration in 0..2_000 {
        let mut document = base.clone();
        let mutations = 1 + rng.below(4);
        for _ in 0..mutations {
            match rng.below(5) {
                // Replace a top-level field with a wrong-typed value.
                0 => {
                    let key = &keys[rng.below(keys.len())];
                    let poison = [
                        json!(null),
                        json!(-1),
                        json!(1.5e308),
                        json!(""),
                        json!([]),
                        json!({}),
                        json!({"nested": [1, 2, 3]}),
                    ];
                    document[key.clone()] = poison[rng.below(poison.len())].clone();
                }
                // Delete a field entirely.
                1 => {
                    let key = &keys[rng.below(keys.len())];
                    document.as_object_mut().unwrap().remove(key);
                }
                // Corrupt the schema version.
                2 => {
                    document["schema_version"] = json!(rng.below(64) as u64);
                }
                // Corrupt a nested numeric leaf.
                3 => {
                    if let Some(object) = document.as_object_mut() {
                        let key = &keys[rng.below(keys.len())];
                        if let Some(nested) = object.get_mut(key).and_then(Value::as_object_mut) {
                            let inner: Vec<String> = nested.keys().cloned().collect();
                            if !inner.is_empty() {
                                let leaf = &inner[rng.below(inner.len())];
                                nested.insert(leaf.clone(), json!(rng.next() as f64));
                            }
                        }
                    }
                }
                // Splice in an oversized string.
                _ => {
                    let key = &keys[rng.below(keys.len())];
                    document[key.clone()] = json!("x".repeat(1 + rng.below(4096)));
                }
            }
        }

        let text = serde_json::to_string(&document).unwrap();
        // The contract: no panic, ever. A rejected document is fine.
        let _ = Capsule::from_json(&text);
        // The same must hold for the migration entry point directly.
        let from = document
            .get("schema_version")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let _ = migrate::migrate_value(document.clone(), from);
        let _ = iteration;
    }
}

#[test]
fn parser_rejects_garbage_without_panicking() {
    let cases = [
        "",
        " ",
        "null",
        "[]",
        "{}",
        "not json at all",
        "{\"schema_version\": 17}",
        "{\"schema_version\": -1}",
        "{\"schema_version\": 1e999}",
        "{\"schema_version\": 4294967296}",
        "{\"schema_version\": \"17\"}",
        "\u{feff}{\"schema_version\": 17}",
        &format!("{{\"schema_version\": {}}}", u64::MAX),
    ];
    for case in cases {
        // Must not panic; an error is the correct outcome for all of these.
        let _ = Capsule::from_json(case);
    }
}

#[test]
fn migration_reaches_current_from_every_historical_version() {
    for version in 1..=CURRENT_SCHEMA_VERSION {
        let document = json!({
            "schema_version": version,
            "toolchain": {},
            "build": {},
            "repeatability": {"runs": 20, "per_metric": {}},
            "metrics": [],
        });
        let migrated = migrate::migrate_value(document, version)
            .unwrap_or_else(|e| panic!("v{version} must migrate: {e}"));
        assert_eq!(
            migrated["schema_version"], CURRENT_SCHEMA_VERSION,
            "v{version} did not reach the current schema"
        );
    }
}

#[test]
fn migration_rejects_future_and_unknown_versions() {
    let future = CURRENT_SCHEMA_VERSION + 1;
    assert!(migrate::migrate_value(json!({"schema_version": future}), future).is_err());
    assert!(migrate::migrate_value(json!({"schema_version": 0}), 0).is_err());
}

#[test]
fn migration_is_idempotent_at_the_current_version() {
    let current = json!({"schema_version": CURRENT_SCHEMA_VERSION, "metrics": []});
    let once = migrate::migrate_value(current.clone(), CURRENT_SCHEMA_VERSION).unwrap();
    assert_eq!(once, current);
    let twice = migrate::migrate_value(once.clone(), CURRENT_SCHEMA_VERSION).unwrap();
    assert_eq!(twice, once);
}
