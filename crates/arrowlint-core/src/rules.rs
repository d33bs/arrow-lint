use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    dataset::{Dataset, Format, SchemaModel},
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    LintConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub default_severity: Severity,
    pub summary: &'static str,
}

pub fn builtin_registry() -> RuleRegistry {
    let mut registry = RuleRegistry::new();
    registry.register(TinyRowGroups);
    registry.register(MissingParquetStatistics);
    registry.register(InconsistentSchemas);
    registry.register(MissingSchemaMetadata);
    registry.register(NonPortableTimestamps);
    registry.register(MixedDictionaryEncodings);
    registry.register(SmallFiles);
    registry.register(UncompressedParquetColumns);
    registry.register(RecognizedExtensionFormat);
    registry
}

struct TinyRowGroups;

impl Rule for TinyRowGroups {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL001",
            name: "tiny-row-groups",
            category: "performance",
            default_severity: Severity::Warning,
            summary: "Parquet row groups should be large enough for efficient scans.",
        }
    }

    fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups
                    .iter()
                    .filter(|group| {
                        group.num_rows > 0
                            && (group.num_rows as u64) < config.rules.min_row_group_rows
                    })
                    .map(|group| {
                        Diagnostic::new(
                            "AL001",
                            Severity::Warning,
                            "performance",
                            format!(
                                "row group {} has {} rows, below configured minimum {}",
                                group.ordinal, group.num_rows, config.rules.min_row_group_rows
                            ),
                        )
                        .with_path(file.path.clone())
                        .with_location(format!("row_group={}", group.ordinal))
                        .with_help("write larger row groups for analytical scans, or lower rules.min_row_group_rows")
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct MissingParquetStatistics;

impl Rule for MissingParquetStatistics {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL002",
            name: "missing-parquet-statistics",
            category: "metadata",
            default_severity: Severity::Warning,
            summary: "Parquet column chunks should include statistics for pruning.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups
                    .iter()
                    .flat_map(|group| {
                        group
                            .columns
                            .iter()
                            .filter(|column| !column.has_statistics)
                            .map(|column| {
                                Diagnostic::new(
                                    "AL002",
                                    Severity::Warning,
                                    "metadata",
                                    format!(
                                        "column `{}` in row group {} has no statistics",
                                        column.path, group.ordinal
                                    ),
                                )
                                .with_path(file.path.clone())
                                .with_location(format!("row_group={},column={}", group.ordinal, column.path))
                                .with_help("enable writer statistics so readers can skip irrelevant pages and row groups")
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct InconsistentSchemas;

impl Rule for InconsistentSchemas {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL003",
            name: "inconsistent-schemas",
            category: "schema",
            default_severity: Severity::Error,
            summary: "Files in one lint target should share a portable schema.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let schemas = dataset
            .files
            .iter()
            .filter_map(|file| {
                file.schema
                    .as_ref()
                    .map(|schema| (file.path.as_str(), schema))
            })
            .collect::<Vec<_>>();
        if schemas.len() <= 1 {
            return Vec::new();
        }

        let baseline = schemas[0].1.fingerprint();
        schemas
            .iter()
            .filter(|(_, schema)| schema.fingerprint() != baseline)
            .map(|(path, schema)| {
                Diagnostic::new(
                    "AL003",
                    Severity::Error,
                    "schema",
                    format!(
                        "schema differs from dataset baseline: {}",
                        summarize_schema(schema)
                    ),
                )
                .with_path((*path).to_string())
                .with_help("align field names, types, ordering, and nullability before treating these files as one dataset")
            })
            .collect()
    }
}

struct MissingSchemaMetadata;

impl Rule for MissingSchemaMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL004",
            name: "missing-schema-metadata",
            category: "metadata",
            default_severity: Severity::Info,
            summary: "Arrow schemas should carry useful producer or domain metadata.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| {
                file.format.is_supported_scanner()
                    && file
                        .schema
                        .as_ref()
                        .is_some_and(|schema| schema.metadata.is_empty())
            })
            .map(|file| {
                Diagnostic::new(
                    "AL004",
                    Severity::Info,
                    "metadata",
                    "schema has no key-value metadata",
                )
                .with_path(file.path.clone())
                .with_help("consider adding producer, domain, CRS, table, or semantic version metadata where useful")
            })
            .collect()
    }
}

struct NonPortableTimestamps;

impl Rule for NonPortableTimestamps {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL005",
            name: "non-portable-timestamps",
            category: "interoperability",
            default_severity: Severity::Warning,
            summary: "Timestamp choices should round-trip across Arrow, Parquet, DuckDB, and table formats.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .flat_map(|file| {
                file.schema
                    .as_ref()
                    .into_iter()
                    .flat_map(|schema| {
                        schema.fields.iter().filter_map(|field| {
                            let data_type = field.data_type.as_str();
                            if data_type.contains("Timestamp(Second") {
                                Some(
                                    Diagnostic::new(
                                        "AL005",
                                        Severity::Warning,
                                        "interoperability",
                                        format!(
                                            "field `{}` uses second-resolution timestamps",
                                            field.name
                                        ),
                                    )
                                    .with_path(file.path.clone())
                                    .with_location(format!("field={}", field.name))
                                    .with_help("prefer microsecond or nanosecond timestamps for Arrow and Parquet interoperability"),
                                )
                            } else if data_type.contains("Timestamp(") && data_type.contains("Some(") {
                                Some(
                                    Diagnostic::new(
                                        "AL005",
                                        Severity::Warning,
                                        "interoperability",
                                        format!("field `{}` has timezone-bearing timestamp type", field.name),
                                    )
                                    .with_path(file.path.clone())
                                    .with_location(format!("field={}", field.name))
                                    .with_help("verify the timezone semantics round-trip through DuckDB, Iceberg, and downstream readers"),
                                )
                            } else {
                                None
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct MixedDictionaryEncodings;

impl Rule for MixedDictionaryEncodings {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL006",
            name: "mixed-dictionary-encodings",
            category: "encoding",
            default_severity: Severity::Warning,
            summary:
                "A Parquet column should not switch dictionary encoding strategy across row groups.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for file in dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
        {
            let mut dictionary_usage: BTreeMap<&str, BTreeSet<bool>> = BTreeMap::new();
            for group in &file.row_groups {
                for column in &group.columns {
                    let uses_dictionary = column
                        .encodings
                        .iter()
                        .any(|encoding| encoding.contains("DICTIONARY"));
                    dictionary_usage
                        .entry(column.path.as_str())
                        .or_default()
                        .insert(uses_dictionary);
                }
            }

            diagnostics.extend(
                dictionary_usage
                    .into_iter()
                    .filter(|(_, states)| states.len() > 1)
                    .map(|(column, _)| {
                        Diagnostic::new(
                            "AL006",
                            Severity::Warning,
                            "encoding",
                            format!(
                                "column `{column}` mixes dictionary and non-dictionary encodings"
                            ),
                        )
                        .with_path(file.path.clone())
                        .with_location(format!("column={column}"))
                        .with_help(
                            "use consistent writer settings, especially for categorical columns",
                        )
                    }),
            );
        }
        diagnostics
    }
}

struct SmallFiles;

impl Rule for SmallFiles {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL007",
            name: "small-files",
            category: "performance",
            default_severity: Severity::Info,
            summary:
                "Datasets with many small files cause avoidable scheduler and metadata overhead.",
        }
    }

    fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic> {
        let small_files = dataset
            .files
            .iter()
            .filter(|file| {
                file.format.is_supported_scanner()
                    && file.size_bytes > 0
                    && file.size_bytes < config.rules.small_file_bytes
            })
            .collect::<Vec<_>>();

        if small_files.len() <= 1 {
            return Vec::new();
        }

        vec![Diagnostic::new(
            "AL007",
            Severity::Info,
            "performance",
            format!(
                "{} files are smaller than {} bytes",
                small_files.len(),
                config.rules.small_file_bytes
            ),
        )
        .with_help(
            "compact small files or lower rules.small_file_bytes for intentionally small datasets",
        )]
    }
}

struct UncompressedParquetColumns;

impl Rule for UncompressedParquetColumns {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL008",
            name: "uncompressed-parquet-columns",
            category: "performance",
            default_severity: Severity::Warning,
            summary: "Parquet columns should usually use a modern compression codec.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups
                    .iter()
                    .flat_map(|group| {
                        group
                            .columns
                            .iter()
                            .filter(|column| column.compression == "UNCOMPRESSED")
                            .map(|column| {
                                Diagnostic::new(
                                    "AL008",
                                    Severity::Warning,
                                    "performance",
                                    format!("column `{}` is uncompressed", column.path),
                                )
                                .with_path(file.path.clone())
                                .with_location(format!("row_group={},column={}", group.ordinal, column.path))
                                .with_help("prefer zstd or snappy unless the dataset is intentionally optimized for another storage layer")
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct RecognizedExtensionFormat;

impl Rule for RecognizedExtensionFormat {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL100",
            name: "recognized-extension-format",
            category: "extension",
            default_severity: Severity::Info,
            summary:
                "ArrowLint recognizes planned format families and routes them to future rule packs.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| !file.format.is_supported_scanner() && file.format != Format::Unknown)
            .map(|file| {
                let format = file.format.as_str();
                Diagnostic::new(
                    "AL100",
                    Severity::Info,
                    "extension",
                    format!("recognized `{format}` input; install or enable its rule pack for deep checks"),
                )
                .with_path(file.path.clone())
                .with_help(format!(
                    "planned rule pack: arrowlint-{format}; current built-ins focus on Arrow IPC, Feather, and Parquet"
                ))
            })
            .collect()
    }
}

fn summarize_schema(schema: &SchemaModel) -> String {
    schema
        .fields
        .iter()
        .map(|field| format!("{}:{}", field.name, field.data_type))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        dataset::{DatasetFile, FieldModel},
        LintConfig,
    };

    use super::*;

    #[test]
    fn inconsistent_schema_rule_flags_drift() {
        let dataset = Dataset {
            schema: None,
            files: vec![
                file_with_schema("a.parquet", "id", "Int64"),
                file_with_schema("b.parquet", "id", "Utf8"),
            ],
        };

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "AL003"));
    }

    #[test]
    fn missing_metadata_rule_is_info() {
        let dataset = Dataset {
            schema: None,
            files: vec![file_with_schema("a.arrow", "id", "Int64")],
        };

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule_id == "AL004")
            .expect("AL004 should fire");
        assert_eq!(diagnostic.severity, Severity::Info);
    }

    fn file_with_schema(path: &str, field_name: &str, data_type: &str) -> DatasetFile {
        DatasetFile {
            path: path.to_string(),
            format: if path.ends_with(".parquet") {
                Format::Parquet
            } else {
                Format::ArrowIpc
            },
            size_bytes: 42,
            num_rows: None,
            schema: Some(SchemaModel {
                fields: vec![FieldModel {
                    name: field_name.to_string(),
                    data_type: data_type.to_string(),
                    nullable: false,
                    metadata: BTreeMap::new(),
                }],
                metadata: BTreeMap::new(),
            }),
            metadata: BTreeMap::new(),
            row_groups: Vec::new(),
        }
    }
}
