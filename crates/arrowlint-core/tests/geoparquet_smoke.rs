use std::{fs::File, sync::Arc};

use arrow::{
    array::{BinaryArray, Int64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use arrowlint_core::{lint_paths, LintConfig, Severity};
use parquet::{
    arrow::ArrowWriter,
    file::{metadata::KeyValue, properties::WriterProperties},
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn accepts_valid_geoparquet_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("valid.parquet");
    write_binary_geoparquet(
        &path,
        json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {
                "geometry": {
                    "encoding": "WKB",
                    "geometry_types": ["Point"],
                    "bbox": [0.0, 1.0, 2.0, 3.0],
                    "edges": "planar"
                }
            }
        })
        .to_string(),
    )?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(
        !report.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.rule_id.as_str(),
            "AL018" | "AL019" | "AL020" | "AL021"
        )),
        "unexpected GeoParquet diagnostics: {:?}",
        report.diagnostics
    );
    Ok(())
}

#[test]
fn reports_invalid_geoparquet_document() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let malformed_path = directory.path().join("malformed.parquet");
    write_binary_geoparquet(&malformed_path, "{".to_string())?;
    let unsupported_path = directory.path().join("unsupported.parquet");
    write_binary_geoparquet(
        &unsupported_path,
        json!({
            "version": "2.0.0",
            "primary_column": "",
            "columns": {}
        })
        .to_string(),
    )?;

    let report = lint_paths(&[malformed_path, unsupported_path], LintConfig::default())?;

    assert!(has_rule(&report, "AL018"));
    Ok(())
}

#[test]
fn reports_duplicate_geoparquet_metadata_keys() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("duplicate.parquet");
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(&b"point"[..])]))],
    )?;
    let geo = json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {
            "geometry": {
                "encoding": "WKB",
                "geometry_types": []
            }
        }
    })
    .to_string();
    let properties = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![
            KeyValue::new("geo".to_string(), Some(geo.clone())),
            KeyValue::new("geo".to_string(), Some(geo)),
        ]))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL018"));
    Ok(())
}

#[test]
fn reports_geoparquet_metadata_without_a_value() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("missing-value.parquet");
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(&b"point"[..])]))],
    )?;
    let properties = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_string(), None)]))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL018"));
    Ok(())
}

#[test]
fn reports_invalid_geoparquet_columns() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("columns.parquet");
    write_integer_geoparquet(
        &path,
        json!({
            "version": "1.1.0",
            "primary_column": "missing",
            "columns": {
                "geometry": {
                    "encoding": "WKB",
                    "geometry_types": ["Point", "Point"]
                },
                "absent": {
                    "encoding": "unknown",
                    "geometry_types": ["CircularString"]
                }
            }
        })
        .to_string(),
    )?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL019"));
    Ok(())
}

#[test]
fn reports_invalid_geoparquet_spatial_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("spatial.parquet");
    write_binary_geoparquet(
        &path,
        json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {
                "geometry": {
                    "encoding": "WKB",
                    "geometry_types": ["Point"],
                    "orientation": "clockwise",
                    "edges": "geodesic",
                    "bbox": [10.0, 1.0, 2.0, 3.0],
                    "covering": {
                        "bbox": {
                            "xmin": ["bounds", "wrong"],
                            "ymin": ["other_bounds", "ymin"],
                            "xmax": ["bounds", "xmax"]
                        }
                    }
                }
            }
        })
        .to_string(),
    )?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL020"));
    Ok(())
}

#[test]
fn reports_missing_geoparquet_pruning_metadata_as_information() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("no-pruning.parquet");
    write_binary_geoparquet(
        &path,
        json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {
                "geometry": {
                    "encoding": "WKB",
                    "geometry_types": []
                }
            }
        })
        .to_string(),
    )?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule_id == "AL021")
        .expect("missing spatial pruning metadata should be reported");

    assert_eq!(diagnostic.severity, Severity::Info);
    Ok(())
}

fn write_binary_geoparquet(path: &std::path::Path, geo: String) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(&b"point"[..])]))],
    )?;
    write_parquet(path, batch, geo)
}

fn write_integer_geoparquet(path: &std::path::Path, geo: String) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Int64,
        true,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))])?;
    write_parquet(path, batch, geo)
}

fn write_parquet(path: &std::path::Path, batch: RecordBatch, geo: String) -> anyhow::Result<()> {
    let properties = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_string(), Some(geo))]))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn has_rule(report: &arrowlint_core::LintReport, rule_id: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == rule_id)
}
