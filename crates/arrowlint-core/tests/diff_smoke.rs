use std::{fs::File, sync::Arc};

use arrow::{
    array::{Float64Array, Int64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use arrowlint_core::{diff_paths, OutputFormat};
use parquet::{
    arrow::ArrowWriter,
    basic::Compression,
    file::{
        metadata::KeyValue,
        properties::{EnabledStatistics, WriterProperties},
    },
};
use tempfile::tempdir;

#[test]
fn compares_parquet_structure_metadata_and_statistics() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let old_path = directory.path().join("old.parquet");
    let new_path = directory.path().join("new.parquet");

    write_parquet(
        &old_path,
        &[1.0, 2.0, 3.0],
        &[10, 20, 30],
        3,
        Compression::UNCOMPRESSED,
        "scope-a",
    )?;
    write_parquet(
        &new_path,
        &[1.0, 2.0, 4.0, 5.0],
        &[10, 20, 31, 40],
        2,
        Compression::ZSTD(Default::default()),
        "scope-b",
    )?;

    let report = diff_paths(&old_path, &new_path)?;

    assert_eq!(report.schema.identical, Some(true));
    assert_eq!(
        report.columns.changed,
        vec![
            "phenotype_score".to_string(),
            "temporary_column".to_string()
        ]
    );
    assert_eq!(report.metadata.changed.len(), 1);
    assert_eq!(report.metadata.changed[0].key, "microscope_model");
    assert!(report.statistics.row_groups_changed);
    assert!(report.statistics.compression_changed);
    assert!(report
        .statistics
        .estimated_scan_cost_change_percent
        .is_some());
    assert!(report.has_changes());

    let text = report.render(OutputFormat::Text)?;
    assert!(text.contains("✓ Schema identical"));
    assert!(text.contains("Column changes"));
    assert!(text.contains("phenotype_score"));
    assert!(text.contains("microscope_model"));
    assert!(text.contains("Row groups changed"));
    assert!(text.contains("Compression changed"));
    assert!(text.contains("Estimated scan cost"));

    let identical = diff_paths(&old_path, &old_path)?;
    assert!(!identical.has_changes());
    assert!(identical.columns.changed.is_empty());
    Ok(())
}

#[test]
fn detects_schema_variants_across_dataset_directories() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let old_directory = directory.path().join("old");
    let new_directory = directory.path().join("new");
    std::fs::create_dir_all(&old_directory)?;
    std::fs::create_dir_all(&new_directory)?;

    write_parquet(
        &old_directory.join("part-1.parquet"),
        &[1.0, 2.0],
        &[10, 20],
        2,
        Compression::UNCOMPRESSED,
        "scope-a",
    )?;
    write_parquet(
        &new_directory.join("part-1.parquet"),
        &[1.0, 2.0],
        &[10, 20],
        2,
        Compression::UNCOMPRESSED,
        "scope-a",
    )?;
    write_parquet_with_extra_column(&new_directory.join("part-2.parquet"))?;

    let report = diff_paths(&old_directory, &new_directory)?;

    assert_eq!(report.schema.identical, Some(false));
    Ok(())
}

fn write_parquet(
    path: &std::path::Path,
    phenotype_scores: &[f64],
    temporary_values: &[i64],
    row_group_rows: usize,
    compression: Compression,
    microscope_model: &str,
) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("phenotype_score", DataType::Float64, false),
        Field::new("temporary_column", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(phenotype_scores.to_vec())),
            Arc::new(Int64Array::from(temporary_values.to_vec())),
        ],
    )?;
    let properties = WriterProperties::builder()
        .set_max_row_group_row_count(Some(row_group_rows))
        .set_compression(compression)
        .set_statistics_enabled(EnabledStatistics::Chunk)
        .set_key_value_metadata(Some(vec![KeyValue::new(
            "microscope_model".to_string(),
            microscope_model.to_string(),
        )]))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(path)?, batch.schema(), Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn write_parquet_with_extra_column(path: &std::path::Path) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("phenotype_score", DataType::Float64, false),
        Field::new("temporary_column", DataType::Int64, false),
        Field::new("new_column", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
            Arc::new(Int64Array::from(vec![10, 20])),
            Arc::new(Int64Array::from(vec![Some(1), None])),
        ],
    )?;
    let mut writer = ArrowWriter::try_new(File::create(path)?, batch.schema(), None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}
