use std::{fs::File, sync::Arc};

use arrow::{
    array::Int64Array,
    datatypes::{DataType, Field, Schema},
    ipc::writer::FileWriter,
    record_batch::RecordBatch,
};
use arrowlint_core::{lint_paths, LintConfig};
use parquet::{arrow::ArrowWriter, file::properties::WriterProperties};
use tempfile::tempdir;

#[test]
fn scans_arrow_ipc_file() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("example.arrow");
    let batch = record_batch()?;
    let mut writer = FileWriter::try_new(File::create(&path)?, &batch.schema())?;
    writer.write(&batch)?;
    writer.finish()?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert_eq!(report.dataset.files.len(), 1);
    assert_eq!(report.dataset.files[0].num_rows, Some(3));
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "AL004"));
    Ok(())
}

#[test]
fn scans_parquet_and_flags_tiny_row_group() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("example.parquet");
    let batch = record_batch()?;
    let properties = WriterProperties::builder()
        .set_max_row_group_row_count(Some(2))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert_eq!(report.dataset.files.len(), 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "AL001"));
    Ok(())
}

fn record_batch() -> anyhow::Result<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    Ok(RecordBatch::try_new(
        schema,
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )?)
}
