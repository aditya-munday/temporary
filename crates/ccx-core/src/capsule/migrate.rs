//! Versioned capsule migrations.
//!
//! The schema only changes by adding a version and a migration from the
//! previous one. Old capsules stay readable forever; they are never silently
//! rewritten on disk.
//!
//! Each step below only adds a safe default. The shape of a migration is:
//!
//! ```ignore
//! 16 => {
//!     // v16 -> v17: `noise_canary` was introduced. Older capsules were
//!     // recorded without a canary, so the field is simply absent.
//!     value["schema_version"] = json!(17);
//!     version = 17;
//! }
//! ```
//!
use super::{CapsuleError, CURRENT_SCHEMA_VERSION, LEGACY_BINARY_DIGEST_UNAVAILABLE};
use serde_json::Value;

/// Read a nested field without panicking, whatever the intermediate types are.
fn get<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

/// Set a nested field, creating the intermediate objects if they are missing.
///
/// Migrations must be *total*: a capsule that arrives malformed is rejected by
/// deserialisation afterwards, never by a panic here. Indexing a
/// `serde_json::Value` that turned out to be a string, number, or array panics,
/// so every nested write goes through this instead of `value["a"]["b"] = ...`.
fn set(value: &mut Value, path: &[&str], new: Value) {
    let Some((head, rest)) = path.split_first() else {
        *value = new;
        return;
    };
    if !value.is_object() {
        *value = Value::Object(serde_json::Map::new());
    }
    let map = value.as_object_mut().expect("just ensured object");
    let entry = map.entry((*head).to_string()).or_insert(Value::Null);
    set(entry, rest, new);
}

/// Borrow a nested object, creating it (and its parents) if needed.
fn object_at<'a>(value: &'a mut Value, path: &[&str]) -> &'a mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(serde_json::Map::new());
    }
    let mut current = value;
    for key in path {
        if !current.is_object() {
            *current = Value::Object(serde_json::Map::new());
        }
        let map = current.as_object_mut().expect("just ensured object");
        current = map.entry((*key).to_string()).or_insert(Value::Null);
    }
    if !current.is_object() {
        *current = Value::Object(serde_json::Map::new());
    }
    current.as_object_mut().expect("just ensured object")
}

/// Migrate a raw capsule document up to the current schema version.
pub fn migrate_value(value: Value, from: u32) -> Result<Value, CapsuleError> {
    if from > CURRENT_SCHEMA_VERSION {
        return Err(CapsuleError::UnsupportedVersion {
            found: from,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let mut value = value;
    let mut version = from;
    while version < CURRENT_SCHEMA_VERSION {
        match version {
            1 => {
                value["schema_version"] = serde_json::json!(2);
                version = 2;
            }
            2 => {
                value["schema_version"] = serde_json::json!(3);
                version = 3;
            }
            3 => {
                value["schema_version"] = serde_json::json!(4);
                version = 4;
            }
            4 => {
                value["schema_version"] = serde_json::json!(5);
                version = 5;
            }
            5 => {
                value["schema_version"] = serde_json::json!(6);
                version = 6;
            }
            6 => {
                value["schema_version"] = serde_json::json!(7);
                version = 7;
            }
            7 => {
                value["schema_version"] = serde_json::json!(8);
                version = 8;
            }
            8 => {
                value["schema_version"] = serde_json::json!(9);
                version = 9;
            }
            9 => {
                set(
                    &mut value,
                    &["toolchain", "environment_fingerprint"],
                    Value::Null,
                );
                value["schema_version"] = serde_json::json!(10);
                version = 10;
            }
            10 => {
                value["schema_version"] = serde_json::json!(11);
                version = 11;
            }
            11 => {
                set(
                    &mut value,
                    &["build", "binary_sha256"],
                    serde_json::json!(LEGACY_BINARY_DIGEST_UNAVAILABLE),
                );
                value["schema_version"] = serde_json::json!(12);
                version = 12;
            }
            12 => {
                value["schema_version"] = serde_json::json!(13);
                version = 13;
            }
            13 => {
                set(&mut value, &["build", "build_repeat"], serde_json::json!(1));
                let runs = get(&value, &["repeatability", "runs"]).and_then(Value::as_u64);
                {
                    let metrics = object_at(&mut value, &["repeatability", "per_metric"]);
                    metrics.remove("build_wall_clock");
                    metrics.remove("build_user_cpu_time");
                    metrics.remove("build_system_cpu_time");
                    if let Some(runs) = runs {
                        for spread in metrics.values_mut() {
                            set(spread, &["samples"], serde_json::json!(runs));
                        }
                    }
                }
                if let Some(metrics) = value["metrics"].as_array_mut() {
                    for metric in metrics {
                        let name = metric["name"].as_str().unwrap_or_default().to_string();
                        if matches!(
                            name.as_str(),
                            "build_wall_clock" | "build_user_cpu_time" | "build_system_cpu_time"
                        ) {
                            if let Some(object) = metric.as_object_mut() {
                                object.remove("uncertainty");
                            }
                            if name == "build_wall_clock" {
                                set(metric, &["kind"], serde_json::json!("measured"));
                            }
                        }
                    }
                }
                value["schema_version"] = serde_json::json!(14);
                version = 14;
            }
            14 => {
                value["schema_version"] = serde_json::json!(15);
                version = 15;
            }
            15 => {
                // v15 -> v16: `applied_tolerance` was introduced. Older capsules
                // never declared one, so the field is simply absent.
                value["schema_version"] = serde_json::json!(16);
                version = 16;
            }
            16 => {
                // v16 -> v17: `noise_canary` was introduced. Older capsules were
                // recorded without a canary, so the field is simply absent
                // rather than fabricated.
                value["schema_version"] = serde_json::json!(17);
                version = 17;
            }
            _ => {
                return Err(CapsuleError::Invalid(format!(
                    "no migration path from schema_version {version}"
                )))
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_version_is_rejected() {
        let err = migrate_value(Value::Null, 99).unwrap_err();
        assert!(matches!(err, CapsuleError::UnsupportedVersion { .. }));
    }

    #[test]
    fn version_one_is_migrated_to_current() {
        let migrated = migrate_value(serde_json::json!({"schema_version": 1}), 1).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn version_seven_is_migrated_to_current() {
        let migrated = migrate_value(serde_json::json!({"schema_version": 7}), 7).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn version_eight_is_migrated_to_current() {
        let migrated = migrate_value(serde_json::json!({"schema_version": 8}), 8).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn version_nine_is_migrated_with_an_unknown_environment_fingerprint() {
        let migrated =
            migrate_value(serde_json::json!({"schema_version": 9, "toolchain": {}}), 9).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated["toolchain"]["environment_fingerprint"].is_null());
    }

    #[test]
    fn version_ten_keeps_legacy_spreads_without_claiming_tail_quantiles() {
        let migrated = migrate_value(
            serde_json::json!({
                "schema_version": 10,
                "repeatability": {
                    "per_metric": {
                        "wall_clock": {
                            "min": 1.0,
                            "max": 3.0,
                            "median": 2.0,
                            "spread_pct": 100.0
                        }
                    }
                }
            }),
            10,
        )
        .unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated["repeatability"]["per_metric"]["wall_clock"]["p95"].is_null());
        assert!(migrated["repeatability"]["per_metric"]["wall_clock"]["p99"].is_null());
        assert_eq!(
            migrated["build"]["binary_sha256"],
            LEGACY_BINARY_DIGEST_UNAVAILABLE
        );
    }

    #[test]
    fn version_eleven_migrates_with_an_unknown_binary_digest() {
        let migrated =
            migrate_value(serde_json::json!({"schema_version": 11, "build": {}}), 11).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert_eq!(
            migrated["build"]["binary_sha256"],
            LEGACY_BINARY_DIGEST_UNAVAILABLE
        );
    }

    #[test]
    fn version_twelve_migrates_without_fabricating_reference_provenance() {
        let migrated = migrate_value(serde_json::json!({"schema_version": 12}), 12).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated.get("reference_build").is_none());
        assert_eq!(migrated["build"]["build_repeat"], 1);
    }

    #[test]
    fn version_thirteen_migrates_with_a_single_honest_build_sample() {
        let migrated = migrate_value(
            serde_json::json!({
                "schema_version": 13,
                "build": {},
                "repeatability": {
                    "runs": 3,
                    "per_metric": {
                        "instructions": {
                            "min": 1,
                            "max": 1,
                            "median": 1,
                            "spread_pct": 0
                        },
                        "build_wall_clock": {
                            "min": 100,
                            "max": 200,
                            "median": 150,
                            "spread_pct": 66.7
                        }
                    }
                },
                "metrics": [{
                    "name": "build_wall_clock",
                    "kind": "indicative",
                    "uncertainty": {"kind": "ci95", "value": 10}
                }]
            }),
            13,
        )
        .unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert_eq!(migrated["build"]["build_repeat"], 1);
        assert!(migrated.get("build_repeatability").is_none());
        assert_eq!(
            migrated["repeatability"]["per_metric"].get("build_wall_clock"),
            None
        );
        assert_eq!(
            migrated["repeatability"]["per_metric"]["instructions"]["samples"],
            3
        );
        assert_eq!(migrated["metrics"][0]["kind"], "measured");
        assert!(migrated["metrics"][0].get("uncertainty").is_none());
    }

    #[test]
    fn version_fourteen_advances_without_fabricating_raw_samples() {
        let migrated = migrate_value(
            serde_json::json!({
                "schema_version": 14,
                "repeatability": {
                    "runs": 20,
                    "per_metric": {
                        "instructions": {
                            "samples": 20,
                            "min": 100,
                            "max": 100,
                            "median": 100,
                            "spread_pct": 0
                        }
                    }
                }
            }),
            14,
        )
        .unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated["repeatability"]["per_metric"]["instructions"]
            .get("values")
            .is_none());
    }

    #[test]
    fn version_fifteen_migrates_without_an_applied_tolerance() {
        let migrated =
            migrate_value(serde_json::json!({"schema_version": 15, "metrics": []}), 15).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated.get("applied_tolerance").is_none());
        assert!(migrated.get("noise_canary").is_none());
    }

    #[test]
    fn version_sixteen_migrates_without_a_noise_canary() {
        let migrated =
            migrate_value(serde_json::json!({"schema_version": 16, "metrics": []}), 16).unwrap();
        assert_eq!(migrated["schema_version"], CURRENT_SCHEMA_VERSION);
        assert!(migrated.get("noise_canary").is_none());
    }

    #[test]
    fn unknown_old_version_is_rejected() {
        let err = migrate_value(Value::Null, 0).unwrap_err();
        assert!(matches!(err, CapsuleError::Invalid(_)));
    }
}
