use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    ArrowIpc,
    Feather,
    Parquet,
    IcebergMetadata,
    LanceDataset,
    Vortex,
    DuckDb,
    Unknown,
}

impl Format {
    pub fn from_path(path: &std::path::Path) -> Self {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with(".parquet") {
            Self::Parquet
        } else if lower.ends_with(".feather") {
            Self::Feather
        } else if lower.ends_with(".arrow") || lower.ends_with(".arrows") || lower.ends_with(".ipc")
        {
            Self::ArrowIpc
        } else if lower.ends_with(".metadata.json") || lower.ends_with(".metadata.json.gz") {
            Self::IcebergMetadata
        } else if lower.ends_with(".lance") || lower.contains(".lance/") {
            Self::LanceDataset
        } else if lower.ends_with(".vortex") || lower.ends_with(".vx") {
            Self::Vortex
        } else if lower.ends_with(".duckdb") || lower.ends_with(".db") {
            Self::DuckDb
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ArrowIpc => "arrow_ipc",
            Self::Feather => "feather",
            Self::Parquet => "parquet",
            Self::IcebergMetadata => "iceberg",
            Self::LanceDataset => "lance",
            Self::Vortex => "vortex",
            Self::DuckDb => "duckdb",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_supported_scanner(&self) -> bool {
        matches!(self, Self::ArrowIpc | Self::Feather | Self::Parquet)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dataset {
    pub files: Vec<DatasetFile>,
    pub schema: Option<SchemaModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetFile {
    pub path: String,
    pub format: Format,
    pub size_bytes: u64,
    pub num_rows: Option<i64>,
    pub schema: Option<SchemaModel>,
    pub metadata: BTreeMap<String, String>,
    pub row_groups: Vec<RowGroupModel>,
    #[serde(skip)]
    pub iceberg_metadata: Option<serde_json::Value>,
    #[serde(skip)]
    pub lance_metadata: Option<LanceMetadata>,
    #[serde(skip)]
    pub vortex_metadata: Option<VortexMetadata>,
    #[serde(skip)]
    pub ipc_metadata: Option<IpcMetadata>,
    #[serde(skip)]
    pub geoparquet_metadata: Option<GeoParquetMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcKind {
    File,
    Stream,
}

#[derive(Debug, Clone)]
pub struct IpcMetadata {
    pub kind: IpcKind,
    pub errors: Vec<String>,
    pub uses_legacy_framing: bool,
    pub has_eos: bool,
    pub message_count: usize,
}

#[derive(Debug, Clone)]
pub struct GeoParquetMetadata {
    pub document: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LanceMetadata {
    pub versions_directory_present: bool,
    pub manifest_count: usize,
    pub selected_manifest_path: Option<String>,
    pub filename_version: Option<u64>,
    pub naming_schemes: Vec<String>,
    pub errors: Vec<String>,
    pub manifest: Option<LanceManifest>,
}

#[derive(Debug, Clone)]
pub struct LanceManifest {
    pub version: u64,
    pub reader_feature_flags: u64,
    pub writer_feature_flags: u64,
    pub max_fragment_id: Option<u32>,
    pub transaction_file: String,
    pub data_format: Option<LanceDataStorageFormat>,
    pub config: BTreeMap<String, String>,
    pub base_paths: Vec<LanceBasePath>,
    pub fragments: Vec<LanceFragment>,
}

#[derive(Debug, Clone)]
pub struct LanceDataStorageFormat {
    pub file_format: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct LanceBasePath {
    pub id: u32,
    pub is_dataset_root: bool,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct LanceFragment {
    pub id: u64,
    pub files: Vec<LanceDataFile>,
    pub deletion_file: Option<LanceDeletionFile>,
    pub physical_rows: u64,
    pub overlay_count: usize,
}

#[derive(Debug, Clone)]
pub struct LanceDataFile {
    pub path: String,
    pub fields: Vec<i32>,
    pub column_indices: Vec<i32>,
    pub file_major_version: u32,
    pub file_minor_version: u32,
    pub file_size_bytes: u64,
    pub base_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct LanceDeletionFile {
    pub file_type: i32,
    pub read_version: u64,
    pub id: u64,
    pub num_deleted_rows: u64,
    pub base_id: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct VortexMetadata {
    pub file_size: u64,
    pub version: Option<u16>,
    pub postscript_length: Option<u16>,
    pub postscript_start: Option<u64>,
    pub errors: Vec<String>,
    pub postscript: Option<VortexPostscript>,
    pub footer: Option<VortexFooter>,
    pub footer_error: Option<String>,
    pub footer_skip_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VortexPostscript {
    pub dtype: Option<VortexSegment>,
    pub layout: Option<VortexSegment>,
    pub statistics: Option<VortexSegment>,
    pub footer: Option<VortexSegment>,
    pub metadata: Vec<VortexUserMetadata>,
}

#[derive(Debug, Clone)]
pub struct VortexUserMetadata {
    pub key: Option<String>,
    pub segment: Option<VortexSegment>,
}

#[derive(Debug, Clone)]
pub struct VortexSegment {
    pub offset: u64,
    pub length: u32,
    pub alignment_exponent: u8,
    pub compression_scheme: Option<u8>,
    pub encrypted: bool,
}

#[derive(Debug, Clone)]
pub struct VortexFooter {
    pub array_ids: Vec<Option<String>>,
    pub layout_ids: Vec<Option<String>>,
    pub segment_specs: Option<Vec<VortexFooterSegment>>,
    pub compression_schemes: Vec<u8>,
    pub encryption_count: usize,
}

#[derive(Debug, Clone)]
pub struct VortexFooterSegment {
    pub offset: u64,
    pub length: u32,
    pub alignment_exponent: u8,
    pub compression_index: u8,
    pub encryption_index: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaModel {
    pub fields: Vec<FieldModel>,
    pub metadata: BTreeMap<String, String>,
}

impl SchemaModel {
    pub fn fingerprint(&self) -> String {
        self.fields
            .iter()
            .map(|field| format!("{}:{}:{}", field.name, field.data_type, field.nullable))
            .collect::<Vec<_>>()
            .join("|")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FieldModel {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowGroupModel {
    pub ordinal: usize,
    pub num_rows: i64,
    pub total_byte_size: i64,
    pub columns: Vec<ColumnChunkModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnChunkModel {
    pub path: String,
    pub physical_type: String,
    pub logical_type: Option<String>,
    pub compression: String,
    pub encodings: Vec<String>,
    pub has_statistics: bool,
    pub statistics: Option<ColumnStatisticsModel>,
    pub num_values: i64,
    pub compressed_size: i64,
    pub uncompressed_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ColumnStatisticsModel {
    pub min_hex: Option<String>,
    pub max_hex: Option<String>,
    pub null_count: Option<u64>,
    pub distinct_count: Option<u64>,
    pub min_is_exact: bool,
    pub max_is_exact: bool,
}
