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
        } else if lower.ends_with(".arrow") || lower.ends_with(".ipc") {
            Self::ArrowIpc
        } else if lower.ends_with("metadata.json") && lower.contains("iceberg") {
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
