use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    dataset::{Dataset, Format},
    diagnostics::{Diagnostic, Severity},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeRule {
    pub rule: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default)]
    pub applies_to: Option<String>,
    pub check: DeclarativeCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeCheck {
    #[serde(default)]
    pub metadata_key: Option<String>,
}

pub fn load_rules(path: &Path) -> Result<Vec<DeclarativeRule>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read declarative rules {}", path.display()))?;
    if raw.trim_start().starts_with('-') {
        serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse declarative rules {}", path.display()))
    } else {
        Ok(vec![serde_yaml::from_str(&raw).with_context(|| {
            format!("failed to parse declarative rule {}", path.display())
        })?])
    }
}

pub fn evaluate(rule: &DeclarativeRule, dataset: &Dataset) -> Vec<Diagnostic> {
    let Some(metadata_key) = &rule.check.metadata_key else {
        return Vec::new();
    };

    let mut diagnostics = Vec::new();
    for file in &dataset.files {
        if !applies_to(&rule.applies_to, &file.format) {
            continue;
        }

        let in_file_metadata = file.metadata.contains_key(metadata_key);
        let in_schema_metadata = file
            .schema
            .as_ref()
            .is_some_and(|schema| schema.metadata.contains_key(metadata_key));

        if !in_file_metadata && !in_schema_metadata {
            let description = rule
                .description
                .clone()
                .unwrap_or_else(|| format!("missing metadata key `{metadata_key}`"));
            diagnostics.push(
                Diagnostic::new(rule.rule.clone(), rule.severity, "metadata", description)
                    .with_path(file.path.clone())
                    .with_help(
                        "add the metadata key or disable this declarative rule for the dataset",
                    ),
            );
        }
    }

    diagnostics
}

fn applies_to(expected: &Option<String>, actual: &Format) -> bool {
    expected
        .as_deref()
        .is_none_or(|expected| expected == actual.as_str())
}
