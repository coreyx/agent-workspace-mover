//! The published JSON Schemas for `--json` output (requirements.md, 22.0.1 and 22.0.4).
//!
//! The schemas are generated from the same types that produce the output, and checked
//! into `schema/` at the repository's root. A test regenerates them and fails if they
//! differ from what's committed. It also fails if a property in a committed schema has
//! disappeared while the schema's version is unchanged, since that breaks readers: a
//! breaking change needs a new version, and so a new file name. To update the committed
//! files after a deliberate change, run the tests with `AWM_WRITE_SCHEMAS=1` set.

use std::collections::BTreeSet;

use awm_core::plan::{PLAN_SCHEMA, Plan, REPORT_SCHEMA, Report};
use serde_json::Value;

/// Each published schema: its file name in `schema/`, and its content.
pub fn schemas() -> Vec<(String, Value)> {
    vec![
        (
            file_name(PLAN_SCHEMA),
            serde_json::to_value(schemars::schema_for!(Plan)).expect("a schema serialises"),
        ),
        (
            file_name(REPORT_SCHEMA),
            serde_json::to_value(schemars::schema_for!(Report)).expect("a schema serialises"),
        ),
    ]
}

/// `awm.plan/1` becomes `awm.plan.v1.json`.
fn file_name(schema: &str) -> String {
    let (name, version) = schema.split_once('/').expect("schema names have a version");
    format!("{name}.v{version}.json")
}

/// Every property's path in `schema`, such as `$defs.Plan.properties.source`.
pub fn property_paths(schema: &Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    collect(schema, String::new(), &mut paths);
    paths
}

/// Adds the property paths under `value`, which sits at `path`, to `paths`.
fn collect(value: &Value, path: String, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let here = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if key == "properties"
                    && let Value::Object(properties) = child
                {
                    for name in properties.keys() {
                        paths.insert(format!("{here}.{name}"));
                    }
                }
                collect(child, here, paths);
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect(item, format!("{path}[{i}]"), paths);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples;
    use std::path::PathBuf;

    /// The repository's `schema/` folder.
    fn schema_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schema")
    }

    // 22.0.4: committed schemas match the code, and nothing is removed without a new version.
    #[test]
    fn committed_schemas_match_and_nothing_was_removed_within_a_version() {
        let dir = schema_dir();
        let write = std::env::var_os("AWM_WRITE_SCHEMAS").is_some();
        for (name, generated) in schemas() {
            let path = dir.join(&name);
            let committed: Option<Value> = std::fs::read(&path)
                .ok()
                .map(|bytes| serde_json::from_slice(&bytes).expect("a committed schema is JSON"));
            if let Some(committed) = &committed {
                let removed: Vec<String> = property_paths(committed)
                    .difference(&property_paths(&generated))
                    .cloned()
                    .collect();
                assert!(
                    removed.is_empty(),
                    "{name} lost {removed:?} without a new version; removing a property breaks readers, so bump the version in awm_core::plan"
                );
            }
            if write {
                std::fs::create_dir_all(&dir).unwrap();
                let mut text = serde_json::to_string_pretty(&generated).unwrap();
                text.push('\n');
                std::fs::write(&path, text).unwrap();
            } else {
                assert_eq!(
                    committed.as_ref(),
                    Some(&generated),
                    "{name} is out of date; run the tests with AWM_WRITE_SCHEMAS=1 and commit schema/"
                );
            }
        }
    }

    // 22.1: output validates against its published schema.
    #[test]
    fn a_full_plan_and_report_validate_against_their_schemas() {
        let instances = [
            serde_json::to_value(samples::plan()).unwrap(),
            serde_json::to_value(samples::report()).unwrap(),
        ];
        for ((name, schema), instance) in schemas().into_iter().zip(instances) {
            let validator = jsonschema::validator_for(&schema)
                .unwrap_or_else(|e| panic!("{name} is a valid schema: {e}"));
            let errors: Vec<String> = validator
                .iter_errors(&instance)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{name}: {errors:?}");
        }
    }

    #[test]
    fn a_damaged_plan_fails_validation() {
        let (_, schema) = schemas().remove(0);
        let mut instance = serde_json::to_value(samples::plan()).unwrap();
        instance["volume_relation"] = Value::String("sideways".into());
        assert!(!jsonschema::is_valid(&schema, &instance));
    }

    #[test]
    fn finds_property_paths_and_names_files_by_version() {
        let schema = serde_json::json!({"properties": {"a": {}, "b": {"properties": {"c": {}}}}, "anyOf": [{"properties": {"d": {}}}]});
        let paths: Vec<String> = property_paths(&schema).into_iter().collect();
        assert_eq!(
            paths,
            [
                "anyOf[0].properties.d",
                "properties.a",
                "properties.b",
                "properties.b.properties.c"
            ]
        );
        assert_eq!(file_name("awm.plan/1"), "awm.plan.v1.json");
    }
}
