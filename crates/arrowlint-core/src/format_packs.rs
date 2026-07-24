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
            status: "extension-ready",
            rule_pack: "arrowlint-iceberg",
            best_practice_focus: &[
                "partition evolution",
                "snapshot metadata",
                "manifest sizing",
                "delete file strategy",
                "Arrow schema compatibility",
            ],
        },
        FormatPack {
            name: "vortex",
            status: "extension-ready",
            rule_pack: "arrowlint-vortex",
            best_practice_focus: &[
                "array encoding selection",
                "statistics availability",
                "chunk sizing",
                "zero-copy interoperability",
            ],
        },
        FormatPack {
            name: "lance",
            status: "extension-ready",
            rule_pack: "arrowlint-lance",
            best_practice_focus: &[
                "fragment sizing",
                "index health",
                "schema evolution",
                "vector column metadata",
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
