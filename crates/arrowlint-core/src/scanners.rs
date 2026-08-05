use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use arrow::{datatypes::SchemaRef, ipc::reader};
use bytes::Bytes;
use flate2::read::GzDecoder;
use parquet::{
    arrow::arrow_reader::ParquetRecordBatchReaderBuilder,
    file::{
        metadata::{KeyValue, ParquetMetaData},
        reader::{FileReader, SerializedFileReader},
    },
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
            Format::LanceDataset if is_remote_url(&path) => {
                return Err(anyhow!(
                    "remote Lance dataset URLs are not supported because they require directory listing: {}",
                    path.display()
                ));
            }
            Format::LanceDataset => crate::lance::scan_dataset(&path)?,
            Format::Vortex if is_remote_url(&path) => {
                let bytes = read_remote_bytes(&path)?;
                crate::vortex::scan_bytes(&path, &bytes)?
            }
            Format::Vortex => crate::vortex::scan_file(&path)?,
            Format::DuckDb => scan_planned_format(&path, format)?,
            Format::Unknown => continue,
        };
        files.push(file);
    }

    if files.is_empty() {
        return Err(anyhow!("no supported arrow-lint inputs found"));
    }

    let schema = files.iter().find_map(|file| file.schema.clone());
    Ok(Dataset { files, schema })
}

fn expand_paths(paths: &[PathBuf], config: &ScanConfig) -> Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for path in paths {
        if is_remote_url(path) {
            expanded.push(path.clone());
            continue;
        }

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
    if is_remote_url(path) {
        let bytes = read_remote_bytes(path)?;
        let size_bytes = bytes.len() as u64;
        let reader = SerializedFileReader::new(bytes.clone())
            .with_context(|| format!("failed to read parquet metadata {}", path.display()))?;
        let arrow_schema = read_parquet_arrow_schema_from_bytes(path, bytes)?;
        return parquet_file_from_metadata(path, size_bytes, reader.metadata(), arrow_schema);
    }

    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let size_bytes = file.metadata()?.len();
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("failed to read parquet metadata {}", path.display()))?;
    let arrow_schema = read_parquet_arrow_schema(path)?;
    parquet_file_from_metadata(path, size_bytes, reader.metadata(), arrow_schema)
}

fn parquet_file_from_metadata(
    path: &Path,
    size_bytes: u64,
    metadata: &ParquetMetaData,
    arrow_schema: SchemaRef,
) -> Result<DatasetFile> {
    let file_metadata = metadata.file_metadata();
    let schema = Some(schema_from_arrow(&arrow_schema));
    let geo_values = file_metadata
        .key_value_metadata()
        .into_iter()
        .flatten()
        .filter(|entry| entry.key == "geo")
        .map(|entry| entry.value.as_deref())
        .collect::<Vec<_>>();
    let geoparquet_metadata = crate::geoparquet::parse_metadata(&geo_values);
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
        vortex_metadata: None,
        ipc_metadata: None,
        geoparquet_metadata,
    })
}

fn read_parquet_arrow_schema_from_bytes(path: &Path, bytes: Bytes) -> Result<SchemaRef> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)
        .with_context(|| format!("failed to decode parquet Arrow schema {}", path.display()))?;
    Ok(builder.schema().clone())
}

fn read_parquet_arrow_schema(path: &Path) -> Result<SchemaRef> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("failed to decode parquet Arrow schema {}", path.display()))?;
    Ok(builder.schema().clone())
}

fn scan_ipc(path: &Path, format: Format) -> Result<DatasetFile> {
    let remote_bytes = if is_remote_url(path) {
        Some(read_remote_bytes(path)?)
    } else {
        None
    };
    let size_bytes = match &remote_bytes {
        Some(bytes) => bytes.len() as u64,
        None => std::fs::metadata(path)?.len(),
    };
    let file = open_seekable_input(path, remote_bytes.as_ref())?;

    let (schema, num_rows, ipc_metadata) = match reader::FileReader::try_new(file, None) {
        Ok(mut ipc_reader) => {
            let schema = schema_from_arrow(&ipc_reader.schema());
            let mut rows = 0_i64;
            for batch in &mut ipc_reader {
                rows += batch?.num_rows() as i64;
            }
            (
                Some(schema),
                Some(rows),
                crate::dataset::IpcMetadata {
                    kind: crate::dataset::IpcKind::File,
                    errors: Vec::new(),
                    uses_legacy_framing: false,
                    has_eos: true,
                    message_count: ipc_reader.num_batches(),
                },
            )
        }
        Err(_) => {
            let mut metadata = match &remote_bytes {
                Some(bytes) => crate::ipc::inspect_stream_bytes(bytes)?,
                None => crate::ipc::inspect_stream(path)?,
            };
            let mut schema = None;
            let mut rows = None;
            if metadata.errors.is_empty() {
                let file = open_seekable_input(path, remote_bytes.as_ref())
                    .with_context(|| format!("failed to reopen {}", path.display()))?;
                match reader::StreamReader::try_new(file, None) {
                    Ok(mut stream_reader) => {
                        schema = Some(schema_from_arrow(&stream_reader.schema()));
                        let mut decoded_rows = 0_i64;
                        for batch in &mut stream_reader {
                            match batch {
                                Ok(batch) => decoded_rows += batch.num_rows() as i64,
                                Err(error) => {
                                    metadata
                                        .errors
                                        .push(format!("Arrow IPC stream data is invalid: {error}"));
                                    break;
                                }
                            }
                        }
                        if metadata.errors.is_empty() {
                            rows = Some(decoded_rows);
                        }
                    }
                    Err(error) => metadata
                        .errors
                        .push(format!("Arrow IPC stream schema is invalid: {error}")),
                }
            }
            (schema, rows, metadata)
        }
    };
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "arrow.ipc_kind".to_string(),
        match ipc_metadata.kind {
            crate::dataset::IpcKind::File => "file",
            crate::dataset::IpcKind::Stream => "stream",
        }
        .to_string(),
    );
    metadata.insert(
        "arrow.ipc_messages".to_string(),
        ipc_metadata.message_count.to_string(),
    );

    Ok(DatasetFile {
        path: path.display().to_string(),
        format,
        size_bytes,
        num_rows,
        schema,
        metadata,
        row_groups: Vec::new(),
        iceberg_metadata: None,
        lance_metadata: None,
        vortex_metadata: None,
        ipc_metadata: Some(ipc_metadata),
        geoparquet_metadata: None,
    })
}

fn scan_iceberg_metadata(path: &Path) -> Result<DatasetFile> {
    let raw = read_input_bytes(path)?;
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
        vortex_metadata: None,
        ipc_metadata: None,
        geoparquet_metadata: None,
    })
}

fn is_gzip_iceberg_metadata(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let input = lower.split(['?', '#']).next().unwrap_or(&lower);
    input.ends_with(".gz.metadata.json") || input.ends_with(".metadata.json.gz")
}

fn scan_planned_format(path: &Path, format: Format) -> Result<DatasetFile> {
    let size_bytes = if is_remote_url(path) {
        read_remote_bytes(path)?.len() as u64
    } else {
        std::fs::metadata(path)?.len()
    };
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
        vortex_metadata: None,
        ipc_metadata: None,
        geoparquet_metadata: None,
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

fn metadata_from_key_values(values: &[KeyValue]) -> BTreeMap<String, String> {
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

fn open_seekable_input(path: &Path, remote_bytes: Option<&Bytes>) -> Result<Box<dyn ReadSeek>> {
    match remote_bytes {
        Some(bytes) => Ok(Box::new(Cursor::new(bytes.to_vec()))),
        None => {
            Ok(Box::new(File::open(path).with_context(|| {
                format!("failed to open {}", path.display())
            })?))
        }
    }
}

fn read_input_bytes(path: &Path) -> Result<Vec<u8>> {
    if is_remote_url(path) {
        Ok(read_remote_bytes(path)?.to_vec())
    } else {
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))
    }
}

fn read_remote_bytes(path: &Path) -> Result<Bytes> {
    match remote_input(path).ok_or_else(|| anyhow!("not a remote URL: {}", path.display()))? {
        RemoteInput::Http(url) => read_http_bytes(url),
        RemoteInput::ObjectStore(url) => read_object_store_bytes(url),
    }
}

fn read_http_bytes(url: &str) -> Result<Bytes> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build remote input HTTP client")?
        .get(url)
        .send()
        .with_context(|| format!("failed to fetch remote input {url}"))?
        .error_for_status()
        .with_context(|| format!("remote input returned unsuccessful status: {url}"))?
        .bytes()
        .with_context(|| format!("failed to read remote input {url}"))
}

fn read_object_store_bytes(url: &str) -> Result<Bytes> {
    let url = url::Url::parse(url).with_context(|| format!("failed to parse object URL {url}"))?;
    let (store, location) = object_store::parse_url_opts(&url, object_store_options_from_env())
        .with_context(|| format!("failed to configure object store for {url}"))?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build object-store runtime")?
        .block_on(async move {
            store
                .get(&location)
                .await
                .with_context(|| format!("failed to fetch object-store input {url}"))?
                .bytes()
                .await
                .with_context(|| format!("failed to read object-store input {url}"))
        })
}

fn object_store_options_from_env() -> Vec<(String, String)> {
    object_store_options_from(std::env::vars())
}

fn object_store_options_from<I, K, V>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let mut options = Vec::new();
    let mut anonymous = false;
    for (key, value) in vars {
        let key = key.as_ref();
        let value = value.into();
        if key == "ARROW_LINT_OBJECT_STORE_ANONYMOUS" {
            anonymous = truthy(&value);
            continue;
        }
        if key.starts_with("AWS_") || key.starts_with("GOOGLE_") || key.starts_with("AZURE_") {
            options.push((key.to_ascii_lowercase(), value));
        } else if key == "SERVICE_ACCOUNT" {
            options.push(("service_account".to_string(), value));
        } else if key == "MSI_ENDPOINT" {
            options.push(("msi_endpoint".to_string(), value));
        }
    }
    options.sort_by(|left, right| left.0.cmp(&right.0));
    if anonymous {
        options.push(("skip_signature".to_string(), "true".to_string()));
    }
    options
}

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_remote_url(path: &Path) -> bool {
    remote_input(path).is_some()
}

fn remote_input(path: &Path) -> Option<RemoteInput<'_>> {
    let value = path.to_str()?;
    if value.starts_with("http://") || value.starts_with("https://") {
        Some(RemoteInput::Http(value))
    } else if is_object_store_url(value) {
        Some(RemoteInput::ObjectStore(value))
    } else {
        None
    }
}

fn is_object_store_url(value: &str) -> bool {
    [
        "s3://", "s3a://", "gs://", "az://", "adl://", "azure://", "abfs://", "abfss://",
    ]
    .iter()
    .any(|scheme| value.starts_with(scheme))
}

enum RemoteInput<'a> {
    Http(&'a str),
    ObjectStore(&'a str),
}

trait ReadSeek: Read + std::io::Seek {}

impl<T: Read + std::io::Seek> ReadSeek for T {}

#[cfg(test)]
mod tests {
    use super::object_store_options_from;

    #[test]
    fn object_store_options_include_auth_environment_and_anonymous_mode() {
        let options = object_store_options_from([
            ("AWS_ACCESS_KEY_ID", "access"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
            ("GOOGLE_SERVICE_ACCOUNT", "/tmp/service-account.json"),
            ("SERVICE_ACCOUNT", "/tmp/alternate-service-account.json"),
            ("AZURE_STORAGE_SAS_TOKEN", "sv=example"),
            (
                "MSI_ENDPOINT",
                "http://169.254.169.254/metadata/identity/oauth2/token",
            ),
            ("ARROW_LINT_OBJECT_STORE_ANONYMOUS", "true"),
            ("UNRELATED", "ignored"),
        ]);

        assert!(options.contains(&("aws_access_key_id".to_string(), "access".to_string())));
        assert!(options.contains(&("aws_secret_access_key".to_string(), "secret".to_string())));
        assert!(options.contains(&(
            "google_service_account".to_string(),
            "/tmp/service-account.json".to_string()
        )));
        assert!(options.contains(&(
            "service_account".to_string(),
            "/tmp/alternate-service-account.json".to_string()
        )));
        assert!(options.contains(&(
            "azure_storage_sas_token".to_string(),
            "sv=example".to_string()
        )));
        assert!(options.contains(&(
            "msi_endpoint".to_string(),
            "http://169.254.169.254/metadata/identity/oauth2/token".to_string()
        )));
        assert_eq!(
            options.last(),
            Some(&("skip_signature".to_string(), "true".to_string()))
        );
        assert!(!options.iter().any(|(key, _)| key == "unrelated"));
    }

    #[test]
    fn object_store_anonymous_mode_requires_truthy_value() {
        let options = object_store_options_from([
            ("ARROW_LINT_OBJECT_STORE_ANONYMOUS", "false"),
            ("AWS_REGION", "us-east-1"),
        ]);

        assert_eq!(
            options,
            vec![("aws_region".to_string(), "us-east-1".to_string())]
        );
    }
}
