use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::{
    config::ScanConfig,
    dataset::{ColumnStatisticsModel, Dataset, FieldModel, SchemaModel},
    report::OutputFormat,
    scanners::scan_paths,
};

const COMPARISON_BASIS: &str = "metadata_and_statistics";
const RESERVED_METADATA_KEYS: &[&str] = &["ARROW:schema"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub comparison_basis: String,
    pub has_changes: bool,
    pub old: DatasetSummary,
    pub new: DatasetSummary,
    pub schema: SchemaDiff,
    pub columns: ColumnDiff,
    pub metadata: MetadataDiff,
    pub statistics: StatisticsDiff,
}

impl DiffReport {
    pub fn has_changes(&self) -> bool {
        self.has_changes
    }

    pub fn render(&self, format: OutputFormat) -> Result<String> {
        match format {
            OutputFormat::Text => Ok(self.render_text()),
            OutputFormat::Json => Ok(serde_json::to_string_pretty(self)?),
            OutputFormat::Sarif => Err(anyhow!(
                "SARIF output is not supported for dataset comparisons"
            )),
        }
    }

    fn render_text(&self) -> String {
        let mut lines = vec![
            "ArrowDiff".to_string(),
            format!("{} → {}", self.old.path, self.new.path),
            String::new(),
        ];

        match self.schema.identical {
            Some(true) => lines.push("✓ Schema identical".to_string()),
            Some(false) => {
                lines.push("✗ Schema changed".to_string());
                render_schema_changes(&mut lines, &self.schema);
            }
            None => lines.push("? Schema unavailable".to_string()),
        }

        lines.push(String::new());
        lines.push("Column changes".to_string());
        if self.columns.changed.is_empty() {
            lines.push("  ✓ No changes detected from column statistics".to_string());
        } else {
            for column in &self.columns.changed {
                lines.push(format!("  - {column}"));
            }
        }

        lines.push(String::new());
        lines.push("Metadata".to_string());
        render_metadata_changes(&mut lines, &self.metadata);

        lines.push(String::new());
        lines.push("Statistics".to_string());
        if self.statistics.row_groups_changed {
            lines.push(format!(
                "  Row groups changed ({} → {})",
                self.statistics.old_row_group_count, self.statistics.new_row_group_count
            ));
        } else {
            lines.push(format!(
                "  ✓ Row groups unchanged ({})",
                self.statistics.old_row_group_count
            ));
        }
        if self.statistics.compression_changed {
            lines.push(format!(
                "  Compression changed ({} → {})",
                display_values(&self.statistics.old_compression),
                display_values(&self.statistics.new_compression)
            ));
        } else {
            lines.push(format!(
                "  ✓ Compression unchanged ({})",
                display_values(&self.statistics.old_compression)
            ));
        }
        match self.statistics.estimated_scan_cost_change_percent {
            Some(percent) => lines.push(format!(
                "  Estimated scan cost {percent:+.1}% ({} → {} bytes)",
                self.statistics.old_estimated_scan_bytes, self.statistics.new_estimated_scan_bytes
            )),
            None => lines.push("  Estimated scan cost unavailable".to_string()),
        }
        lines.push(String::new());
        lines.push(
            "Comparison uses file metadata and column statistics; it does not prove row-level equality."
                .to_string(),
        );
        lines.push(String::new());
        lines.join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSummary {
    pub path: String,
    pub formats: Vec<String>,
    pub file_count: usize,
    pub row_count: Option<i64>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDiff {
    pub identical: Option<bool>,
    pub old_variant_count: usize,
    pub new_variant_count: usize,
    pub order_changed: bool,
    pub added: Vec<FieldModel>,
    pub removed: Vec<FieldModel>,
    pub changed: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub name: String,
    pub old: FieldModel,
    pub new: FieldModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDiff {
    pub changed: Vec<String>,
    pub evidence: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataDiff {
    pub added: Vec<MetadataValue>,
    pub removed: Vec<MetadataValue>,
    pub changed: Vec<MetadataChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataChange {
    pub key: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticsDiff {
    pub old_file_count: usize,
    pub new_file_count: usize,
    pub file_count_changed: bool,
    pub old_row_count: Option<i64>,
    pub new_row_count: Option<i64>,
    pub row_count_changed: bool,
    pub old_row_group_count: usize,
    pub new_row_group_count: usize,
    pub row_groups_changed: bool,
    pub old_compression: Vec<String>,
    pub new_compression: Vec<String>,
    pub compression_changed: bool,
    pub old_estimated_scan_bytes: u64,
    pub new_estimated_scan_bytes: u64,
    pub estimated_scan_cost_change_percent: Option<f64>,
    pub scan_cost_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ColumnEvidence {
    num_values: i64,
    statistics: Option<ColumnStatisticsModel>,
}

pub fn diff_paths(old_path: &Path, new_path: &Path) -> Result<DiffReport> {
    diff_paths_with_config(old_path, new_path, &ScanConfig::default())
}

pub fn diff_paths_with_config(
    old_path: &Path,
    new_path: &Path,
    config: &ScanConfig,
) -> Result<DiffReport> {
    let old = scan_paths(&[PathBuf::from(old_path)], config)?;
    let new = scan_paths(&[PathBuf::from(new_path)], config)?;

    Ok(compare_datasets(old_path, &old, new_path, &new))
}

fn compare_datasets(old_path: &Path, old: &Dataset, new_path: &Path, new: &Dataset) -> DiffReport {
    let old_summary = summarize_dataset(old_path, old);
    let new_summary = summarize_dataset(new_path, new);
    let schema = compare_schemas(old, new);
    let columns = compare_columns(old, new);
    let metadata = compare_metadata(old, new);
    let statistics = compare_statistics(old, new);
    let has_changes = report_has_changes(&schema, &columns, &metadata, &statistics);

    DiffReport {
        comparison_basis: COMPARISON_BASIS.to_string(),
        has_changes,
        old: old_summary,
        new: new_summary,
        schema,
        columns,
        metadata,
        statistics,
    }
}

fn report_has_changes(
    schema: &SchemaDiff,
    columns: &ColumnDiff,
    metadata: &MetadataDiff,
    statistics: &StatisticsDiff,
) -> bool {
    schema.identical == Some(false)
        || !columns.changed.is_empty()
        || !metadata.added.is_empty()
        || !metadata.removed.is_empty()
        || !metadata.changed.is_empty()
        || statistics.file_count_changed
        || statistics.row_count_changed
        || statistics.row_groups_changed
        || statistics.compression_changed
        || statistics.scan_cost_changed
}

fn summarize_dataset(path: &Path, dataset: &Dataset) -> DatasetSummary {
    let formats = dataset
        .files
        .iter()
        .map(|file| file.format.as_str().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    DatasetSummary {
        path: path.display().to_string(),
        formats,
        file_count: dataset.files.len(),
        row_count: total_rows(dataset),
        size_bytes: dataset.files.iter().map(|file| file.size_bytes).sum(),
    }
}

fn compare_schemas(old_dataset: &Dataset, new_dataset: &Dataset) -> SchemaDiff {
    let old_variants = schema_variants(old_dataset);
    let new_variants = schema_variants(new_dataset);
    let (Some(old), Some(new)) = (old_dataset.schema.as_ref(), new_dataset.schema.as_ref()) else {
        return SchemaDiff {
            identical: None,
            old_variant_count: old_variants.len(),
            new_variant_count: new_variants.len(),
            order_changed: false,
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
        };
    };

    let old_fields = fields_by_name(old);
    let new_fields = fields_by_name(new);
    let added = new_fields
        .iter()
        .filter(|(name, _)| !old_fields.contains_key(*name))
        .map(|(_, field)| (*field).clone())
        .collect();
    let removed = old_fields
        .iter()
        .filter(|(name, _)| !new_fields.contains_key(*name))
        .map(|(_, field)| (*field).clone())
        .collect();
    let changed = old_fields
        .iter()
        .filter_map(|(name, old_field)| {
            let new_field = new_fields.get(name)?;
            (**old_field != **new_field).then(|| FieldChange {
                name: name.clone(),
                old: (*old_field).clone(),
                new: (**new_field).clone(),
            })
        })
        .collect();
    let old_order = old
        .fields
        .iter()
        .map(|field| &field.name)
        .collect::<Vec<_>>();
    let new_order = new
        .fields
        .iter()
        .map(|field| &field.name)
        .collect::<Vec<_>>();

    SchemaDiff {
        identical: Some(old_variants == new_variants),
        old_variant_count: old_variants.len(),
        new_variant_count: new_variants.len(),
        order_changed: old_order != new_order,
        added,
        removed,
        changed,
    }
}

fn schema_variants(dataset: &Dataset) -> BTreeSet<Vec<FieldModel>> {
    dataset
        .files
        .iter()
        .filter_map(|file| file.schema.as_ref())
        .map(|schema| schema.fields.clone())
        .collect()
}

fn fields_by_name(schema: &SchemaModel) -> BTreeMap<String, &FieldModel> {
    schema
        .fields
        .iter()
        .map(|field| (field.name.clone(), field))
        .collect()
}

fn compare_columns(old: &Dataset, new: &Dataset) -> ColumnDiff {
    let old_evidence = column_evidence(old);
    let new_evidence = column_evidence(new);
    let mut changed = Vec::new();
    let mut evidence = BTreeMap::new();

    for (name, old_values) in &old_evidence {
        let Some(new_values) = new_evidence.get(name) else {
            continue;
        };
        if old_values != new_values {
            changed.push(name.clone());
            evidence.insert(name.clone(), vec!["statistics".to_string()]);
        }
    }

    ColumnDiff { changed, evidence }
}

fn column_evidence(dataset: &Dataset) -> BTreeMap<String, Vec<ColumnEvidence>> {
    let mut evidence = BTreeMap::<String, Vec<ColumnEvidence>>::new();
    for file in &dataset.files {
        for row_group in &file.row_groups {
            for column in &row_group.columns {
                evidence
                    .entry(column.path.clone())
                    .or_default()
                    .push(ColumnEvidence {
                        num_values: column.num_values,
                        statistics: column.statistics.clone(),
                    });
            }
        }
    }
    for values in evidence.values_mut() {
        values.sort();
    }
    evidence
}

fn compare_metadata(old: &Dataset, new: &Dataset) -> MetadataDiff {
    let old_metadata = merged_metadata(old);
    let new_metadata = merged_metadata(new);
    let added = new_metadata
        .iter()
        .filter(|(key, _)| !old_metadata.contains_key(*key))
        .map(|(key, value)| MetadataValue {
            key: key.clone(),
            value: value.clone(),
        })
        .collect();
    let removed = old_metadata
        .iter()
        .filter(|(key, _)| !new_metadata.contains_key(*key))
        .map(|(key, value)| MetadataValue {
            key: key.clone(),
            value: value.clone(),
        })
        .collect();
    let changed = old_metadata
        .iter()
        .filter_map(|(key, old_value)| {
            let new_value = new_metadata.get(key)?;
            (old_value != new_value).then(|| MetadataChange {
                key: key.clone(),
                old_value: old_value.clone(),
                new_value: new_value.clone(),
            })
        })
        .collect();

    MetadataDiff {
        added,
        removed,
        changed,
    }
}

fn merged_metadata(dataset: &Dataset) -> BTreeMap<String, String> {
    let mut values = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(schema) = &dataset.schema {
        for (key, value) in &schema.metadata {
            if !RESERVED_METADATA_KEYS.contains(&key.as_str()) {
                values.entry(key.clone()).or_default().insert(value.clone());
            }
        }
    }
    for file in &dataset.files {
        for (key, value) in &file.metadata {
            if !RESERVED_METADATA_KEYS.contains(&key.as_str()) {
                values.entry(key.clone()).or_default().insert(value.clone());
            }
        }
    }
    values
        .into_iter()
        .map(|(key, values)| (key, values.into_iter().collect::<Vec<_>>().join(" | ")))
        .collect()
}

fn compare_statistics(old: &Dataset, new: &Dataset) -> StatisticsDiff {
    let old_row_group_count = row_group_count(old);
    let new_row_group_count = row_group_count(new);
    let old_compression = compression_values(old);
    let new_compression = compression_values(new);
    let old_estimated_scan_bytes = estimated_scan_bytes(old);
    let new_estimated_scan_bytes = estimated_scan_bytes(new);
    let estimated_scan_cost_change_percent =
        percentage_change(old_estimated_scan_bytes, new_estimated_scan_bytes);

    StatisticsDiff {
        old_file_count: old.files.len(),
        new_file_count: new.files.len(),
        file_count_changed: old.files.len() != new.files.len(),
        old_row_count: total_rows(old),
        new_row_count: total_rows(new),
        row_count_changed: total_rows(old) != total_rows(new),
        old_row_group_count,
        new_row_group_count,
        row_groups_changed: row_group_signatures(old) != row_group_signatures(new),
        compression_changed: old_compression != new_compression,
        old_compression,
        new_compression,
        old_estimated_scan_bytes,
        new_estimated_scan_bytes,
        scan_cost_changed: old_estimated_scan_bytes != new_estimated_scan_bytes,
        estimated_scan_cost_change_percent,
    }
}

fn total_rows(dataset: &Dataset) -> Option<i64> {
    dataset
        .files
        .iter()
        .map(|file| file.num_rows)
        .try_fold(0_i64, |total, rows| rows.map(|rows| total + rows))
}

fn row_group_count(dataset: &Dataset) -> usize {
    dataset.files.iter().map(|file| file.row_groups.len()).sum()
}

fn row_group_signatures(dataset: &Dataset) -> Vec<(i64, i64, Vec<i64>)> {
    let mut signatures = dataset
        .files
        .iter()
        .flat_map(|file| &file.row_groups)
        .map(|row_group| {
            (
                row_group.num_rows,
                row_group.total_byte_size,
                row_group
                    .columns
                    .iter()
                    .map(|column| column.num_values)
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    signatures.sort();
    signatures
}

fn compression_values(dataset: &Dataset) -> Vec<String> {
    dataset
        .files
        .iter()
        .flat_map(|file| &file.row_groups)
        .flat_map(|row_group| &row_group.columns)
        .map(|column| column.compression.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn estimated_scan_bytes(dataset: &Dataset) -> u64 {
    dataset
        .files
        .iter()
        .map(|file| {
            let compressed_bytes = file
                .row_groups
                .iter()
                .flat_map(|row_group| &row_group.columns)
                .filter_map(|column| u64::try_from(column.compressed_size).ok())
                .sum::<u64>();
            if compressed_bytes == 0 {
                file.size_bytes
            } else {
                compressed_bytes
            }
        })
        .sum()
}

fn percentage_change(old: u64, new: u64) -> Option<f64> {
    if old == 0 {
        return None;
    }
    Some(((new as f64 - old as f64) / old as f64) * 100.0)
}

fn render_schema_changes(lines: &mut Vec<String>, schema: &SchemaDiff) {
    if schema.old_variant_count != schema.new_variant_count {
        lines.push(format!(
            "  Schema variants changed ({} → {})",
            schema.old_variant_count, schema.new_variant_count
        ));
    }
    if !schema.added.is_empty() {
        lines.push("  Added:".to_string());
        lines.extend(
            schema
                .added
                .iter()
                .map(|field| format!("    + {} ({})", field.name, field.data_type)),
        );
    }
    if !schema.removed.is_empty() {
        lines.push("  Removed:".to_string());
        lines.extend(
            schema
                .removed
                .iter()
                .map(|field| format!("    - {} ({})", field.name, field.data_type)),
        );
    }
    if !schema.changed.is_empty() {
        lines.push("  Changed:".to_string());
        lines.extend(
            schema
                .changed
                .iter()
                .map(|field| format!("    ~ {}", field.name)),
        );
    }
    if schema.order_changed {
        lines.push("  Column order changed".to_string());
    }
}

fn render_metadata_changes(lines: &mut Vec<String>, metadata: &MetadataDiff) {
    if metadata.added.is_empty() && metadata.removed.is_empty() && metadata.changed.is_empty() {
        lines.push("  ✓ No metadata changes".to_string());
        return;
    }
    if !metadata.added.is_empty() {
        lines.push("  Added:".to_string());
        lines.extend(
            metadata
                .added
                .iter()
                .map(|entry| format!("    + {}", entry.key)),
        );
    }
    if !metadata.removed.is_empty() {
        lines.push("  Removed:".to_string());
        lines.extend(
            metadata
                .removed
                .iter()
                .map(|entry| format!("    - {}", entry.key)),
        );
    }
    if !metadata.changed.is_empty() {
        lines.push("  Changed:".to_string());
        lines.extend(
            metadata
                .changed
                .iter()
                .map(|entry| format!("    ~ {}", entry.key)),
        );
    }
}

fn display_values(values: &[String]) -> String {
    if values.is_empty() {
        "n/a".to_string()
    } else {
        values.join(", ")
    }
}
