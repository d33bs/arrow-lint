use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use arrow::{datatypes::SchemaRef, ipc::reader};
use parquet::{
    arrow::arrow_reader::ParquetRecordBatchReaderBuilder,
    file::reader::{FileReader, SerializedFileReader},
};
use walkdir::WalkDir;

use crate::{
    config::ScanConfig,
    dataset::{
        ColumnChunkModel, Dataset, DatasetFile, FieldModel, Format, RowGroupModel, SchemaModel,
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
            Format::IcebergMetadata | Format::LanceDataset | Format::Vortex | Format::DuckDb => {
                scan_planned_format(&path, format)?
            }
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
        if path.is_file() {
            expanded.push(path.clone());
            continue;
        }

        if path.is_dir() {
            if config.recursive {
                for entry in WalkDir::new(path).follow_links(config.follow_links) {
                    let entry =
                        entry.with_context(|| format!("failed to walk {}", path.display()))?;
                    if entry.file_type().is_file() {
                        expanded.push(entry.path().to_path_buf());
                    }
                }
            } else {
                for entry in std::fs::read_dir(path)
                    .with_context(|| format!("failed to read {}", path.display()))?
                {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
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
                    ColumnChunkModel {
                        path: column.column_path().string(),
                        physical_type: column.column_type().to_string(),
                        logical_type: None,
                        compression: column.compression().to_string(),
                        encodings: column.encodings().map(|value| value.to_string()).collect(),
                        has_statistics: column.statistics().is_some(),
                        num_values: column.num_values(),
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
    })
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
