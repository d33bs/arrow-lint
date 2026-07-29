use std::{fs, io::Write, path::Path};

use arrowlint_core::{lint_paths, LintConfig};
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn scans_standard_iceberg_metadata_without_placeholder_diagnostic() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    write_metadata(&path, &valid_metadata())?;

    let report = lint_paths(&[path], LintConfig::default())?;
    let rule_ids = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(report.dataset.files[0].format.as_str(), "iceberg");
    assert!(!rule_ids.contains(&"AL100"));
    assert!(
        !rule_ids.iter().any(|rule_id| rule_id.starts_with("AL1")),
        "unexpected Iceberg diagnostics: {rule_ids:?}"
    );
    Ok(())
}

#[test]
fn accepts_legacy_v1_iceberg_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("v1.metadata.json");
    let metadata = json!({
        "format-version": 1,
        "location": "s3://warehouse/db/table",
        "last-updated-ms": 1_700_000_000_000_i64,
        "last-column-id": 1,
        "schema": {
            "type": "struct",
            "fields": [
                {"id": 1, "name": "id", "required": true, "type": "long"}
            ]
        },
        "partition-spec": [],
        "properties": {},
        "current-snapshot-id": -1,
        "snapshots": [],
        "snapshot-log": [],
        "metadata-log": []
    });
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id.starts_with("AL1")));
    Ok(())
}

#[test]
fn accepts_v3_iceberg_metadata_with_row_lineage() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("v3.metadata.json");
    let mut metadata = valid_metadata();
    metadata["format-version"] = json!(3);
    metadata["next-row-id"] = json!(2);
    metadata["snapshots"][0]["first-row-id"] = json!(0);
    metadata["snapshots"][0]["added-rows"] = json!(2);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id.starts_with("AL1")));
    Ok(())
}

#[test]
fn accepts_v3_multi_source_partition_and_sort_fields() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("v3-multi-source.metadata.json");
    let mut metadata = valid_metadata();
    metadata["format-version"] = json!(3);
    metadata["next-row-id"] = json!(2);
    metadata["snapshots"][0]["first-row-id"] = json!(0);
    metadata["snapshots"][0]["added-rows"] = json!(2);
    metadata["partition-specs"] = json!([{
        "spec-id": 1,
        "fields": [{
            "source-ids": [1, 2],
            "field-id": 1000,
            "name": "combined_partition",
            "transform": "bucket[16]"
        }]
    }]);
    metadata["default-spec-id"] = json!(1);
    metadata["last-partition-id"] = json!(1000);
    metadata["sort-orders"] = json!([{
        "order-id": 1,
        "fields": [{
            "source-ids": [1, 2],
            "transform": "identity",
            "direction": "asc",
            "null-order": "nulls-first"
        }]
    }]);
    metadata["default-sort-order-id"] = json!(1);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(!has_rule(&report, "AL106"));
    Ok(())
}

#[test]
fn accepts_implicit_unsorted_iceberg_sort_order() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("unsorted.metadata.json");
    let mut metadata = valid_metadata();
    metadata["sort-orders"] = json!([]);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "AL102" || diagnostic.rule_id == "AL103"));
    Ok(())
}

#[test]
fn accepts_metadata_only_commit_after_current_snapshot() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("metadata-update.metadata.json");
    let mut metadata = valid_metadata();
    metadata["last-updated-ms"] = json!(1_700_000_001_000_i64);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(!has_rule(&report, "AL105"));
    Ok(())
}

#[test]
fn scans_gzip_compressed_iceberg_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.gz.metadata.json");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&serde_json::to_vec(&valid_metadata())?)?;
    fs::write(&path, encoder.finish()?)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert_eq!(report.dataset.files[0].format.as_str(), "iceberg");
    assert!(!has_rule(&report, "AL100"));
    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id.starts_with("AL1")));
    Ok(())
}

#[test]
fn rejects_non_object_iceberg_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    fs::write(&path, b"[]")?;

    let error = lint_paths(&[path], LintConfig::default()).unwrap_err();

    assert!(error
        .to_string()
        .contains("Iceberg metadata root must be a JSON object"));
    Ok(())
}

#[test]
fn reports_unsupported_iceberg_format_version() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    let mut metadata = valid_metadata();
    metadata["format-version"] = json!(4);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL101"));
    Ok(())
}

#[test]
fn reports_missing_required_iceberg_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    let mut metadata = valid_metadata();
    metadata.as_object_mut().unwrap().remove("table-uuid");
    metadata["location"] = json!("relative/table");
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL102"));
    Ok(())
}

#[test]
fn reports_broken_iceberg_references_and_identifiers() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    let mut metadata = valid_metadata();
    metadata["current-schema-id"] = json!(99);
    metadata["default-spec-id"] = json!(99);
    metadata["default-sort-order-id"] = json!(99);
    metadata["current-snapshot-id"] = json!(99);
    metadata["refs"]["main"]["snapshot-id"] = json!(11);
    let duplicate_schema = metadata["schemas"][0].clone();
    metadata["schemas"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_schema);
    let duplicate_snapshot = metadata["snapshots"][0].clone();
    metadata["snapshots"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_snapshot);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL103"));
    assert!(has_rule(&report, "AL104"));
    Ok(())
}

#[test]
fn reports_invalid_iceberg_snapshots_and_field_ids() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    let mut metadata = valid_metadata();
    metadata["last-sequence-number"] = json!(0);
    metadata["snapshots"][0]
        .as_object_mut()
        .unwrap()
        .remove("manifest-list");
    metadata["snapshots"][0]["summary"] = json!({});
    metadata["schemas"][0]["fields"][1]["id"] = json!(1);
    metadata["last-column-id"] = json!(0);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL105"));
    assert!(has_rule(&report, "AL106"));
    Ok(())
}

#[test]
fn reports_missing_nested_iceberg_field_ids() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("missing-field-id.metadata.json");
    let mut metadata = valid_metadata();
    metadata["schemas"][0]["fields"][1]
        .as_object_mut()
        .unwrap()
        .remove("id");
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL106"));
    Ok(())
}

#[test]
fn reports_conflicting_partition_field_id_reuse() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("partition-evolution.metadata.json");
    let mut metadata = valid_metadata();
    metadata["partition-specs"] = json!([
        {
            "spec-id": 0,
            "fields": [{
                "source-id": 1,
                "field-id": 1000,
                "name": "id",
                "transform": "identity"
            }]
        },
        {
            "spec-id": 1,
            "fields": [{
                "source-id": 2,
                "field-id": 1000,
                "name": "value",
                "transform": "identity"
            }]
        }
    ]);
    metadata["default-spec-id"] = json!(1);
    metadata["last-partition-id"] = json!(1000);
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(has_rule(&report, "AL106"));
    Ok(())
}

#[test]
fn reports_large_iceberg_metadata_history() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("00001-abc.metadata.json");
    let mut metadata = valid_metadata();
    metadata["metadata-log"] = json!([
        {
            "timestamp-ms": 1_699_999_998_000_i64,
            "metadata-file": "s3://warehouse/db/table/metadata/00000.metadata.json"
        },
        {
            "timestamp-ms": 1_699_999_999_000_i64,
            "metadata-file": "s3://warehouse/db/table/metadata/00001.metadata.json"
        }
    ]);
    let mut config = LintConfig::default();
    config.rules.iceberg_max_metadata_log_entries = 1;
    write_metadata(&path, &metadata)?;

    let report = lint_paths(&[path], config)?;

    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule_id == "AL107")
        .unwrap();
    assert_eq!(diagnostic.category, "maintenance");
    assert_eq!(diagnostic.severity, arrowlint_core::Severity::Info);
    Ok(())
}

fn has_rule(report: &arrowlint_core::LintReport, rule_id: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == rule_id)
}

fn write_metadata(path: &Path, metadata: &Value) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(metadata)?)?;
    Ok(())
}

fn valid_metadata() -> Value {
    json!({
        "format-version": 2,
        "table-uuid": "2f9c5734-5f6d-4f41-8762-c09483e48af8",
        "location": "s3://warehouse/db/table",
        "last-sequence-number": 1,
        "last-updated-ms": 1_700_000_000_000_i64,
        "last-column-id": 2,
        "schemas": [{
            "type": "struct",
            "schema-id": 0,
            "fields": [
                {"id": 1, "name": "id", "required": true, "type": "long"},
                {"id": 2, "name": "value", "required": false, "type": "string"}
            ]
        }],
        "current-schema-id": 0,
        "partition-specs": [{"spec-id": 0, "fields": []}],
        "default-spec-id": 0,
        "last-partition-id": 999,
        "properties": {},
        "current-snapshot-id": 10,
        "snapshots": [{
            "sequence-number": 1,
            "snapshot-id": 10,
            "timestamp-ms": 1_700_000_000_000_i64,
            "manifest-list": "s3://warehouse/db/table/metadata/snap-10.avro",
            "summary": {"operation": "append"},
            "schema-id": 0
        }],
        "snapshot-log": [{
            "timestamp-ms": 1_700_000_000_000_i64,
            "snapshot-id": 10
        }],
        "metadata-log": [],
        "sort-orders": [{"order-id": 0, "fields": []}],
        "default-sort-order-id": 0,
        "refs": {
            "main": {"snapshot-id": 10, "type": "branch"}
        }
    })
}
