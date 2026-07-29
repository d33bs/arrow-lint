use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FormatPack {
    pub name: &'static str,
    pub status: &'static str,
    pub rule_pack: &'static str,
    pub best_practice_focus: &'static [&'static str],
}

pub fn known_format_packs() -> Vec<FormatPack> {
    vec![
        FormatPack {
            name: "parquet",
            status: "built-in",
            rule_pack: "arrowlint-core",
            best_practice_focus: &[
                "row group sizing",
                "statistics coverage",
                "compression",
                "encoding consistency",
                "portable logical types",
            ],
        },
        FormatPack {
            name: "arrow_ipc",
            status: "built-in",
            rule_pack: "arrowlint-core",
            best_practice_focus: &[
                "schema metadata",
                "field nullability",
                "timestamp portability",
                "dictionary compatibility",
            ],
        },
        FormatPack {
            name: "iceberg",
            status: "built-in-metadata",
            rule_pack: "arrowlint-core",
            best_practice_focus: &[
                "format version and required fields",
                "schema and partition field IDs",
                "partition evolution",
                "snapshot metadata",
                "reference integrity",
                "metadata history maintenance",
            ],
        },
        FormatPack {
            name: "vortex",
            status: "built-in-metadata",
            rule_pack: "arrowlint-core",
            best_practice_focus: &[
                "file and postscript integrity",
                "segment bounds and alignment",
                "encoding and layout registries",
                "statistics and dtype portability",
            ],
        },
        FormatPack {
            name: "lance",
            status: "built-in-metadata",
            rule_pack: "arrowlint-core",
            best_practice_focus: &[
                "fragment sizing",
                "manifest and feature compatibility",
                "data and deletion references",
                "version retention",
            ],
        },
        FormatPack {
            name: "duckdb",
            status: "extension-ready",
            rule_pack: "arrowlint-duckdb",
            best_practice_focus: &[
                "Arrow export compatibility",
                "nested type round-tripping",
                "timestamp and timezone semantics",
                "Parquet writer settings",
            ],
        },
    ]
}
