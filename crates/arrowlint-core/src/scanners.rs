use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use arrow::{datatypes::SchemaRef, ipc::reader};
use flate2::read::GzDecoder;
use parquet::{
    arrow::arrow_reader::ParquetRecordBatchReaderBuilder,
    file::reader::{FileReader, SerializedFileReader},
};
use walkdir::WalkDir;

use crate::{
    config::ScanConfig,
    dataset::{
        ColumnChunkModel, ColumnStatisticsModel, Dataset, DatasetFile, FieldModel, Format,
        RowGroupModel, SchemaModel,
    },
};

pub fn scan_paths(paths: &[PathBuf], config: &ScanConfig) -> Result<Dataset> {
    let mut files = Vec::new();
    for path in expand_paths(paths, config)? {
        let format = Format::from_path(&path);
        if format == Format::Unknown {
            continue;
        }

        let file = match format {
            Format::Parquet => scan_parquet(&path)?,
            Format::ArrowIpc | Format::Feather => scan_ipc(&path, format)?,
            Format::IcebergMetadata => scan_iceberg_metadata(&path)?,
            Format::LanceDataset => crate::lance::scan_dataset(&path)?,
            Format::Vortex | Format::DuckDb => scan_planned_format(&path, format)?,
            Format::Unknown => continue,
        };
        files.push(file);
    }

    if files.is_empty() {
        return Err(anyhow!("no supported ArrowLint inputs found"));
    }

    let schema = files.iter().find_map(|file| file.schema.clone());
    Ok(Dataset { files, schema })
}

fn expand_paths(paths: &[PathBuf], config: &ScanConfig) -> Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for path in paths {
        if let Some(root) = lance_dataset_root(path) {
            if path.exists() {
                expanded.push(root);
                continue;
            }
        }

        if path.is_file() {
            expanded.push(path.clone());
            continue;
        }

        if path.is_dir() {
            if config.recursive {
                let mut entries = WalkDir::new(path)
                    .follow_links(config.follow_links)
                    .into_iter();
                while let Some(entry) = entries.next() {
                    let entry =
                        entry.with_context(|| format!("failed to walk {}", path.display()))?;
                    if entry.file_type().is_dir() && is_lance_dataset_root(entry.path()) {
                        expanded.push(entry.path().to_path_buf());
                        entries.skip_current_dir();
                        continue;
                    }
                    if entry.file_type().is_file() {
                        expanded.push(entry.path().to_path_buf());
                    }
                }
            } else {
                for entry in std::fs::read_dir(path)
                    .with_context(|| format!("failed to read {}", path.display()))?
                {
                    let entry = entry?;
                    let file_type = entry.file_type()?;
                    if file_type.is_file()
                        || (file_type.is_dir() && is_lance_dataset_root(&entry.path()))
                    {
                        expanded.push(entry.path());
                    }
                }
            }
            continue;
        }

        return Err(anyhow!("input path does not exist: {}", path.display()));
    }

    expanded.sort();
    expanded.dedup();
    Ok(expanded)
}

fn lance_dataset_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| is_lance_dataset_root(ancestor))
        .map(Path::to_path_buf)
}

fn is_lance_dataset_root(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".lance"))
}

fn scan_parquet(path: &Path) -> Result<DatasetFile> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let size_bytes = file.metadata()?.len();
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("failed to read parquet metadata {}", path.display()))?;
    let metadata = reader.metadata();
    let file_metadata = metadata.file_metadata();
    let arrow_schema = read_parquet_arrow_schema(path)?;
    let schema = Some(schema_from_arrow(&arrow_schema));
    let key_values = file_metadata
        .key_value_metadata()
        .map(|values| metadata_from_key_values(values))
        .unwrap_or_default();

    let row_groups = (0..metadata.num_row_groups())
        .map(|index| {
            let group = metadata.row_group(index);
            let columns = (0..group.num_columns())
                .map(|column_index| {
                    let column = group.column(column_index);
                    let statistics = column.statistics().map(|statistics| ColumnStatisticsModel {
                        min_hex: statistics.min_bytes_opt().map(bytes_to_hex),
                        max_hex: statistics.max_bytes_opt().map(bytes_to_hex),
                        null_count: statistics.null_count_opt(),
                        distinct_count: statistics.distinct_count_opt(),
                        min_is_exact: statistics.min_is_exact(),
                        max_is_exact: statistics.max_is_exact(),
                    });
                    ColumnChunkModel {
                        path: column.column_path().string(),
                        physical_type: column.column_type().to_string(),
                        logical_type: None,
                        compression: column.compression().to_string(),
                        encodings: column.encodings().map(|value| value.to_string()).collect(),
                        has_statistics: statistics.is_some(),
                        statistics,
                        num_values: column.num_values(),
                        compressed_size: column.compressed_size(),
                        uncompressed_size: column.uncompressed_size(),
                    }
                })
                .collect();
            RowGroupModel {
                ordinal: index,
                num_rows: group.num_rows(),
                total_byte_size: group.total_byte_size(),
                columns,
            }
        })
        .collect();

    Ok(DatasetFile {
        path: path.display().to_string(),
        format: Format::Parquet,
        size_bytes,
        num_rows: Some(file_metadata.num_rows()),
        schema,
        metadata: key_values,
        row_groups,
        iceberg_metadata: None,
        lance_metadata: None,
    })
}

fn read_parquet_arrow_schema(path: &Path) -> Result<SchemaRef> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("failed to decode parquet Arrow schema {}", path.display()))?;
    Ok(builder.schema().clone())
}

fn scan_ipc(path: &Path, format: Format) -> Result<DatasetFile> {
    let size_bytes = std::fs::metadata(path)?.len();
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;

    let (schema, num_rows) = match reader::FileReader::try_new(file, None) {
        Ok(mut ipc_reader) => {
            let schema = schema_from_arrow(&ipc_reader.schema());
            let mut rows = 0_i64;
            for batch in &mut ipc_reader {
                rows += batch?.num_rows() as i64;
            }
            (schema, Some(rows))
        }
        Err(_) => {
            let file =
                File::open(path).with_context(|| format!("failed to reopen {}", path.display()))?;
            let mut stream_reader = reader::StreamReader::try_new(file, None)
                .with_context(|| format!("failed to read Arrow IPC stream {}", path.display()))?;
            let schema = schema_from_arrow(&stream_reader.schema());
            let mut rows = 0_i64;
            for batch in &mut stream_reader {
                rows += batch?.num_rows() as i64;
            }
            (schema, Some(rows))
        }
    };

    Ok(DatasetFile {
        path: path.display().to_string(),
        format,
        size_bytes,
        num_rows,
        schema: Some(schema),
        metadata: BTreeMap::new(),
        row_groups: Vec::new(),
        iceberg_metadata: None,
        lance_metadata: None,
    })
}

fn scan_iceberg_metadata(path: &Path) -> Result<DatasetFile> {
    let raw = std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let decoded = if is_gzip_iceberg_metadata(path) {
        let mut decoded = Vec::new();
        GzDecoder::new(raw.as_slice())
            .read_to_end(&mut decoded)
            .with_context(|| format!("failed to decompress Iceberg metadata {}", path.display()))?;
        decoded
    } else {
        raw.clone()
    };
    let metadata: serde_json::Value = serde_json::from_slice(&decoded)
        .with_context(|| format!("failed to parse Iceberg metadata {}", path.display()))?;
    if !metadata.is_object() {
        return Err(anyhow!(
            "Iceberg metadata root must be a JSON object: {}",
            path.display()
        ));
    }
    Ok(DatasetFile {
        path: path.display().to_string(),
        format: Format::IcebergMetadata,
        size_bytes: raw.len() as u64,
        num_rows: None,
        schema: None,
        metadata: BTreeMap::new(),
        row_groups: Vec::new(),
        iceberg_metadata: Some(metadata),
        lance_metadata: None,
    })
}

fn is_gzip_iceberg_metadata(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    lower.ends_with(".gz.metadata.json") || lower.ends_with(".metadata.json.gz")
}

fn scan_planned_format(path: &Path, format: Format) -> Result<DatasetFile> {
    let size_bytes = std::fs::metadata(path)?.len();
    Ok(DatasetFile {
        path: path.display().to_string(),
        format,
        size_bytes,
        num_rows: None,
        schema: None,
        metadata: BTreeMap::new(),
        row_groups: Vec::new(),
        iceberg_metadata: None,
        lance_metadata: None,
    })
}

fn schema_from_arrow(schema: &SchemaRef) -> SchemaModel {
    SchemaModel {
        fields: schema
            .fields()
            .iter()
            .map(|field| FieldModel {
                name: field.name().clone(),
                data_type: field.data_type().to_string(),
                nullable: field.is_nullable(),
                metadata: field.metadata().clone().into_iter().collect(),
            })
            .collect(),
        metadata: schema.metadata().clone().into_iter().collect(),
    }
}

fn metadata_from_key_values(
    values: &[parquet::file::metadata::KeyValue],
) -> BTreeMap<String, String> {
    values
        .iter()
        .filter_map(|entry| {
            entry
                .value
                .as_ref()
                .map(|value| (entry.key.clone(), value.clone()))
        })
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
