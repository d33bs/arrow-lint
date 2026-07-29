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
    registry.register(DeprecatedParquetPhysicalTypes);
    registry.register(DeprecatedParquetEncodings);
    registry.register(DeprecatedParquetCompression);
    registry.register(InvalidParquetStatistics);
    registry.register(InconsistentParquetRowCounts);
    registry.register(MissingParquetNullCounts);
    registry.register(InvalidParquetSizeMetadata);
    crate::iceberg::register_rules(&mut registry);
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

struct DeprecatedParquetPhysicalTypes;

impl Rule for DeprecatedParquetPhysicalTypes {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL009",
            name: "deprecated-parquet-physical-types",
            category: "interoperability",
            default_severity: Severity::Warning,
            summary: "Parquet files should not use deprecated physical types.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups.iter().flat_map(|group| {
                    group
                        .columns
                        .iter()
                        .filter(|column| column.physical_type == "INT96")
                        .map(|column| {
                            Diagnostic::new(
                                "AL009",
                                Severity::Warning,
                                "interoperability",
                                format!(
                                    "column `{}` uses the deprecated INT96 physical type",
                                    column.path
                                ),
                            )
                            .with_path(file.path.clone())
                            .with_location(format!(
                                "row_group={},column={}",
                                group.ordinal, column.path
                            ))
                            .with_help(
                                "rewrite timestamps as INT64 with a TIMESTAMP logical annotation",
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect()
    }
}

struct DeprecatedParquetEncodings;

impl Rule for DeprecatedParquetEncodings {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL010",
            name: "deprecated-parquet-encodings",
            category: "interoperability",
            default_severity: Severity::Warning,
            summary: "Parquet files should not use deprecated encodings.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups.iter().flat_map(|group| {
                    group.columns.iter().filter_map(|column| {
                        let deprecated = column
                            .encodings
                            .iter()
                            .filter(|encoding| {
                                matches!(encoding.as_str(), "PLAIN_DICTIONARY" | "BIT_PACKED")
                            })
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        if deprecated.is_empty() {
                            return None;
                        }

                        Some(
                            Diagnostic::new(
                                "AL010",
                                Severity::Warning,
                                "interoperability",
                                format!(
                                    "column `{}` uses deprecated encoding(s): {}",
                                    column.path,
                                    deprecated.into_iter().collect::<Vec<_>>().join(", ")
                                ),
                            )
                            .with_path(file.path.clone())
                            .with_location(format!(
                                "row_group={},column={}",
                                group.ordinal, column.path
                            ))
                            .with_help(
                                "rewrite with RLE_DICTIONARY for dictionary data and RLE for levels",
                            ),
                        )
                    })
                })
            })
            .collect()
    }
}

struct DeprecatedParquetCompression;

impl Rule for DeprecatedParquetCompression {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL011",
            name: "deprecated-parquet-compression",
            category: "interoperability",
            default_severity: Severity::Warning,
            summary: "Parquet files should not use deprecated compression codecs.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups.iter().flat_map(|group| {
                    group
                        .columns
                        .iter()
                        .filter(|column| column.compression == "LZ4")
                        .map(|column| {
                            Diagnostic::new(
                                "AL011",
                                Severity::Warning,
                                "interoperability",
                                format!("column `{}` uses the deprecated LZ4 codec", column.path),
                            )
                            .with_path(file.path.clone())
                            .with_location(format!(
                                "row_group={},column={}",
                                group.ordinal, column.path
                            ))
                            .with_help(
                                "rewrite with LZ4_RAW, ZSTD, or SNAPPY for broad reader support",
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect()
    }
}

struct InvalidParquetStatistics;

impl Rule for InvalidParquetStatistics {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL012",
            name: "invalid-parquet-statistics",
            category: "correctness",
            default_severity: Severity::Error,
            summary: "Parquet statistic counts must not exceed the column value count.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for file in dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
        {
            for group in &file.row_groups {
                for column in &group.columns {
                    let Ok(num_values) = u64::try_from(column.num_values) else {
                        continue;
                    };
                    let Some(statistics) = &column.statistics else {
                        continue;
                    };

                    for (name, count) in [
                        ("null_count", statistics.null_count),
                        ("distinct_count", statistics.distinct_count),
                    ] {
                        let Some(count) = count else {
                            continue;
                        };
                        if count > num_values {
                            diagnostics.push(
                                Diagnostic::new(
                                    "AL012",
                                    Severity::Error,
                                    "correctness",
                                    format!(
                                        "{name} {} exceeds num_values {num_values} for column `{}`",
                                        count, column.path
                                    ),
                                )
                                .with_path(file.path.clone())
                                .with_location(format!(
                                    "row_group={},column={}",
                                    group.ordinal, column.path
                                ))
                                .with_help(
                                    "rewrite the file with correct statistics before relying on predicate pruning",
                                ),
                            );
                        }
                    }
                }
            }
        }
        diagnostics
    }
}

struct InconsistentParquetRowCounts;

impl Rule for InconsistentParquetRowCounts {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL013",
            name: "inconsistent-parquet-row-counts",
            category: "correctness",
            default_severity: Severity::Error,
            summary: "The Parquet file row count must equal the sum of its row groups.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .filter_map(|file| {
                let expected = file.num_rows?;
                if file.row_groups.iter().any(|group| group.num_rows < 0) {
                    return None;
                }
                let Some(actual) = file
                    .row_groups
                    .iter()
                    .map(|group| group.num_rows)
                    .try_fold(0_i64, i64::checked_add)
                else {
                    return Some(
                        Diagnostic::new(
                            "AL013",
                            Severity::Error,
                            "correctness",
                            "row-group row counts overflow the Parquet metadata range",
                        )
                        .with_path(file.path.clone())
                        .with_help(
                            "rewrite the file so file-level and row-group row counts are consistent",
                        ),
                    );
                };
                if expected == actual {
                    return None;
                }

                Some(
                    Diagnostic::new(
                        "AL013",
                        Severity::Error,
                        "correctness",
                        format!(
                            "file metadata reports {expected} rows but row groups sum to {actual}"
                        ),
                    )
                    .with_path(file.path.clone())
                    .with_help(
                        "rewrite the file so file-level and row-group row counts are consistent",
                    ),
                )
            })
            .collect()
    }
}

struct MissingParquetNullCounts;

impl Rule for MissingParquetNullCounts {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL014",
            name: "missing-parquet-null-counts",
            category: "metadata",
            default_severity: Severity::Warning,
            summary: "Parquet statistics should explicitly include null counts.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
            .flat_map(|file| {
                file.row_groups.iter().flat_map(|group| {
                    group
                        .columns
                        .iter()
                        .filter(|column| {
                            column
                                .statistics
                                .as_ref()
                                .is_some_and(|statistics| statistics.null_count.is_none())
                        })
                        .map(|column| {
                            Diagnostic::new(
                                "AL014",
                                Severity::Warning,
                                "metadata",
                                format!(
                                    "statistics for column `{}` omit null_count",
                                    column.path
                                ),
                            )
                            .with_path(file.path.clone())
                            .with_location(format!(
                                "row_group={},column={}",
                                group.ordinal, column.path
                            ))
                            .with_help(
                                "configure the writer to record null_count, including when it is zero",
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect()
    }
}

struct InvalidParquetSizeMetadata;

impl Rule for InvalidParquetSizeMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL015",
            name: "invalid-parquet-size-metadata",
            category: "correctness",
            default_severity: Severity::Error,
            summary: "Parquet row, value, and byte counts must not be negative.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for file in dataset
            .files
            .iter()
            .filter(|file| file.format == Format::Parquet)
        {
            if file.num_rows.is_some_and(|num_rows| num_rows < 0) {
                diagnostics.push(
                    Diagnostic::new(
                        "AL015",
                        Severity::Error,
                        "correctness",
                        "file row count is negative",
                    )
                    .with_path(file.path.clone())
                    .with_help("rewrite the file with valid non-negative metadata counts"),
                );
            }

            for group in &file.row_groups {
                for (name, value) in [
                    ("num_rows", group.num_rows),
                    ("total_byte_size", group.total_byte_size),
                ] {
                    if value < 0 {
                        diagnostics.push(
                            Diagnostic::new(
                                "AL015",
                                Severity::Error,
                                "correctness",
                                format!("row group {} has negative {name}: {value}", group.ordinal),
                            )
                            .with_path(file.path.clone())
                            .with_location(format!("row_group={}", group.ordinal))
                            .with_help("rewrite the file with valid non-negative metadata counts"),
                        );
                    }
                }

                for column in &group.columns {
                    for (name, value) in [
                        ("num_values", column.num_values),
                        ("compressed_size", column.compressed_size),
                        ("uncompressed_size", column.uncompressed_size),
                    ] {
                        if value < 0 {
                            diagnostics.push(
                                Diagnostic::new(
                                    "AL015",
                                    Severity::Error,
                                    "correctness",
                                    format!(
                                        "column `{}` has negative {name}: {value}",
                                        column.path
                                    ),
                                )
                                .with_path(file.path.clone())
                                .with_location(format!(
                                    "row_group={},column={}",
                                    group.ordinal, column.path
                                ))
                                .with_help(
                                    "rewrite the file with valid non-negative metadata counts",
                                ),
                            );
                        }
                    }
                }
            }
        }
        diagnostics
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
            .filter(|file| {
                !file.format.is_supported_scanner()
                    && file.format != Format::IcebergMetadata
                    && file.format != Format::Unknown
            })
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
                    "planned rule pack: arrowlint-{format}; current built-ins focus on Arrow IPC, Feather, Parquet, and Iceberg metadata"
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
        dataset::{
            ColumnChunkModel, ColumnStatisticsModel, DatasetFile, FieldModel, RowGroupModel,
        },
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

    #[test]
    fn deprecated_parquet_features_are_reported() {
        let dataset = parquet_dataset(
            Some(1),
            vec![row_group(
                1,
                128,
                vec![column(
                    "created_at",
                    "INT96",
                    "LZ4",
                    &["PLAIN_DICTIONARY", "BIT_PACKED"],
                    1,
                    64,
                    128,
                    Some(statistics(Some(0), Some(1))),
                )],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        for rule_id in ["AL009", "AL010", "AL011"] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule_id),
                "{rule_id} should report the deprecated Parquet feature"
            );
        }
    }

    #[test]
    fn impossible_parquet_statistics_are_errors() {
        let dataset = parquet_dataset(
            Some(10),
            vec![row_group(
                10,
                128,
                vec![column(
                    "id",
                    "INT64",
                    "ZSTD",
                    &["PLAIN"],
                    10,
                    64,
                    128,
                    Some(statistics(Some(11), Some(12))),
                )],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());
        let invalid_statistics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule_id == "AL012")
            .collect::<Vec<_>>();

        assert_eq!(invalid_statistics.len(), 2);
        assert!(invalid_statistics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Error));
    }

    #[test]
    fn inconsistent_parquet_row_total_is_an_error() {
        let dataset = parquet_dataset(
            Some(10),
            vec![row_group(4, 64, vec![]), row_group(5, 64, vec![])],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule_id == "AL013")
            .expect("AL013 should report inconsistent row totals");

        assert_eq!(diagnostic.severity, Severity::Error);
    }

    #[test]
    fn overflowing_parquet_row_total_is_an_error() {
        let dataset = parquet_dataset(
            Some(i64::MAX),
            vec![row_group(i64::MAX, 64, vec![]), row_group(1, 64, vec![])],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "AL013"));
    }

    #[test]
    fn missing_parquet_null_count_is_reported() {
        let dataset = parquet_dataset(
            Some(1),
            vec![row_group(
                1,
                128,
                vec![column(
                    "id",
                    "INT64",
                    "ZSTD",
                    &["PLAIN"],
                    1,
                    64,
                    128,
                    Some(statistics(None, Some(1))),
                )],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "AL014"));
    }

    #[test]
    fn negative_parquet_metadata_is_an_error() {
        let dataset = parquet_dataset(
            Some(1),
            vec![row_group(
                -1,
                -128,
                vec![column(
                    "id",
                    "INT64",
                    "ZSTD",
                    &["PLAIN"],
                    -1,
                    -64,
                    -128,
                    None,
                )],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());
        let invalid_metadata = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule_id == "AL015")
            .collect::<Vec<_>>();

        assert_eq!(invalid_metadata.len(), 5);
        assert!(invalid_metadata
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Error));
    }

    #[test]
    fn valid_modern_parquet_metadata_does_not_trigger_new_rules() {
        let dataset = parquet_dataset(
            Some(10),
            vec![row_group(
                10,
                128,
                vec![column(
                    "id",
                    "INT64",
                    "LZ4_RAW",
                    &["PLAIN", "RLE_DICTIONARY", "RLE"],
                    10,
                    64,
                    128,
                    Some(statistics(Some(10), Some(10))),
                )],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());
        let new_rule_ids = diagnostics
            .iter()
            .filter_map(|diagnostic| {
                let numeric_id = diagnostic.rule_id.strip_prefix("AL")?.parse::<u16>().ok()?;
                (9..=15)
                    .contains(&numeric_id)
                    .then_some(diagnostic.rule_id.as_str())
            })
            .collect::<Vec<_>>();

        assert!(
            new_rule_ids.is_empty(),
            "unexpected rules: {new_rule_ids:?}"
        );
    }

    #[test]
    fn absent_statistics_do_not_duplicate_missing_null_count_diagnostic() {
        let dataset = parquet_dataset(
            Some(1),
            vec![row_group(
                1,
                128,
                vec![column("id", "INT64", "ZSTD", &["PLAIN"], 1, 64, 128, None)],
            )],
        );

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "AL002"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "AL014"));
    }

    #[test]
    fn negative_file_row_count_is_an_error() {
        let dataset = parquet_dataset(Some(-1), Vec::new());

        let diagnostics = builtin_registry().check(&dataset, &LintConfig::default());

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "AL015" && diagnostic.message == "file row count is negative"
        }));
    }

    #[test]
    fn builtin_rule_ids_are_unique_and_include_format_validity_rules() {
        let metadata = builtin_registry().metadata();
        let ids = metadata.iter().map(|rule| rule.id).collect::<BTreeSet<_>>();

        assert_eq!(ids.len(), metadata.len());
        for rule_id in [
            "AL009", "AL010", "AL011", "AL012", "AL013", "AL014", "AL015", "AL101", "AL102",
            "AL103", "AL104", "AL105", "AL106", "AL107",
        ] {
            assert!(ids.contains(rule_id), "{rule_id} should be registered");
        }
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
            iceberg_metadata: None,
        }
    }

    fn parquet_dataset(num_rows: Option<i64>, row_groups: Vec<RowGroupModel>) -> Dataset {
        Dataset {
            schema: None,
            files: vec![DatasetFile {
                path: "example.parquet".to_string(),
                format: Format::Parquet,
                size_bytes: 256,
                num_rows,
                schema: None,
                metadata: BTreeMap::new(),
                row_groups,
                iceberg_metadata: None,
            }],
        }
    }

    fn row_group(
        num_rows: i64,
        total_byte_size: i64,
        columns: Vec<ColumnChunkModel>,
    ) -> RowGroupModel {
        RowGroupModel {
            ordinal: 0,
            num_rows,
            total_byte_size,
            columns,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn column(
        path: &str,
        physical_type: &str,
        compression: &str,
        encodings: &[&str],
        num_values: i64,
        compressed_size: i64,
        uncompressed_size: i64,
        statistics: Option<ColumnStatisticsModel>,
    ) -> ColumnChunkModel {
        ColumnChunkModel {
            path: path.to_string(),
            physical_type: physical_type.to_string(),
            logical_type: None,
            compression: compression.to_string(),
            encodings: encodings
                .iter()
                .map(|encoding| (*encoding).to_string())
                .collect(),
            has_statistics: statistics.is_some(),
            statistics,
            num_values,
            compressed_size,
            uncompressed_size,
        }
    }

    fn statistics(null_count: Option<u64>, distinct_count: Option<u64>) -> ColumnStatisticsModel {
        ColumnStatisticsModel {
            min_hex: None,
            max_hex: None,
            null_count,
            distinct_count,
            min_is_exact: false,
            max_is_exact: false,
        }
    }
}
