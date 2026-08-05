use std::{
    fs::{self, File},
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
};

use arrow::{
    array::Int64Array,
    datatypes::{DataType, Field, Schema},
    ipc::writer::FileWriter,
    record_batch::RecordBatch,
};
use arrowlint_core::{lint_paths, LintConfig};
use parquet::{arrow::ArrowWriter, basic::Compression, file::properties::WriterProperties};
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

#[test]
fn scans_remote_http_parquet() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("remote.parquet");
    let batch = record_batch()?;
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), None)?;
    writer.write(&batch)?;
    writer.close()?;
    let (url, server) = serve_file_once(path)?;

    let report = lint_paths(&[PathBuf::from(&url)], LintConfig::default());
    if report.is_ok() {
        server
            .join()
            .map_err(|_| anyhow::anyhow!("server thread panicked"))??;
    }
    let report = report?;

    assert_eq!(report.dataset.files.len(), 1);
    assert_eq!(report.dataset.files[0].path, url);
    assert_eq!(report.dataset.files[0].num_rows, Some(3));
    Ok(())
}

#[test]
fn scans_deprecated_parquet_compression() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("deprecated-compression.parquet");
    let batch = record_batch()?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::LZ4)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;

    let report = lint_paths(&[path], LintConfig::default())?;

    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "AL011"));
    Ok(())
}

#[test]
fn disabled_parquet_rule_is_removed_from_report() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("deprecated-compression.parquet");
    let batch = record_batch()?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::LZ4)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    let mut config = LintConfig::default();
    config.rules.disabled.insert("AL011".to_string());

    let report = lint_paths(&[path], config)?;

    assert!(!report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "AL011"));
    Ok(())
}

#[test]
fn only_selected_rules_are_included_in_report() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("deprecated-compression.parquet");
    let batch = record_batch()?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::LZ4)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    let mut config = LintConfig::default();
    config.rules.only.insert("AL011".to_string());

    let report = lint_paths(&[path], config)?;

    assert!(!report.diagnostics.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.rule_id == "AL011"));
    Ok(())
}

#[test]
fn disabled_rules_override_only_selected_rules() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("deprecated-compression.parquet");
    let batch = record_batch()?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::LZ4)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    let mut config = LintConfig::default();
    config.rules.only.insert("AL011".to_string());
    config.rules.disabled.insert("AL011".to_string());

    let report = lint_paths(&[path], config)?;

    assert!(report.diagnostics.is_empty());
    Ok(())
}

fn serve_file_once(path: PathBuf) -> anyhow::Result<(String, JoinHandle<anyhow::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/remote.parquet?signature=example");
    let handle = thread::spawn(move || -> anyhow::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request)?;
        let bytes = fs::read(path)?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )?;
        stream.write_all(&bytes)?;
        Ok(())
    });
    Ok((url, handle))
}

fn record_batch() -> anyhow::Result<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    Ok(RecordBatch::try_new(
        schema,
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )?)
}
