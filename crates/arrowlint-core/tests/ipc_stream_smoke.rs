use std::{fs, sync::Arc};

use arrow::{
    array::Int64Array,
    datatypes::{DataType, Field, Schema},
    ipc::writer::StreamWriter,
    record_batch::RecordBatch,
};
use arrowlint_core::{lint_paths, LintConfig};
use tempfile::tempdir;

#[test]
fn scans_arrow_ipc_stream_files() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("example.arrows");
    write_stream(&path)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert_eq!(report.dataset.files.len(), 1);
    assert_eq!(report.dataset.files[0].format.as_str(), "arrow_ipc");
    assert_eq!(report.dataset.files[0].num_rows, Some(3));
    assert!(!has_rule(&report, "AL016"));
    assert!(!has_rule(&report, "AL017"));
    Ok(())
}

#[test]
fn accepts_stream_files_without_optional_eos_marker() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("no-eos.arrows");
    write_stream(&path)?;
    let mut bytes = fs::read(&path)?;
    assert_eq!(
        &bytes[bytes.len() - 8..],
        &[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]
    );
    bytes.truncate(bytes.len() - 8);
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(!has_rule(&report, "AL016"));
    Ok(())
}

#[test]
fn reports_legacy_ipc_stream_framing() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("legacy.arrows");
    write_stream(&path)?;
    let mut bytes = fs::read(&path)?;
    assert_eq!(&bytes[..4], &[0xff; 4]);
    bytes.drain(..4);
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL017"));
    Ok(())
}

#[test]
fn reports_malformed_ipc_stream_framing() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("malformed.arrows");
    fs::write(&path, [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f])?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL016"));
    Ok(())
}

#[test]
fn reports_bytes_after_ipc_stream_eos() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("trailing.arrows");
    write_stream(&path)?;
    let mut bytes = fs::read(&path)?;
    bytes.extend_from_slice(b"trailing");
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL016"));
    Ok(())
}

fn write_stream(path: &std::path::Path) -> anyhow::Result<()> {
    let batch = record_batch()?;
    let mut bytes = Vec::new();
    let mut writer = StreamWriter::try_new(&mut bytes, &batch.schema())?;
    writer.write(&batch)?;
    writer.finish()?;
    drop(writer);
    fs::write(path, bytes)?;
    Ok(())
}

fn record_batch() -> anyhow::Result<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    Ok(RecordBatch::try_new(
        schema,
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )?)
}

fn has_rule(report: &arrowlint_core::LintReport, rule_id: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == rule_id)
}
