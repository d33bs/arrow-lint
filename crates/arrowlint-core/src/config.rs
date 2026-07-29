use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{declarative::DeclarativeRule, Severity};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LintConfig {
    pub scan: ScanConfig,
    pub rules: RuleConfig,
    pub output: OutputConfig,
    #[serde(skip)]
    pub declarative_rules: Vec<DeclarativeRule>,
}

impl LintConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let mut config: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        config.load_rule_files(base_dir)?;
        Ok(config)
    }

    pub fn load_rule_files(&mut self, base_dir: &Path) -> Result<()> {
        let mut loaded = Vec::new();
        for path in &self.rules.declarative_rule_files {
            let path = if path.is_absolute() {
                path.clone()
            } else {
                base_dir.join(path)
            };
            loaded.extend(crate::declarative::load_rules(&path)?);
        }
        self.declarative_rules = loaded;
        Ok(())
    }

    pub fn is_rule_enabled(&self, rule_id: &str) -> bool {
        (self.rules.only.is_empty() || self.rules.only.contains(rule_id))
            && !self.rules.disabled.contains(rule_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub recursive: bool,
    pub follow_links: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            recursive: true,
            follow_links: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleConfig {
    pub min_row_group_rows: u64,
    pub small_file_bytes: u64,
    pub iceberg_max_snapshots: usize,
    pub iceberg_max_metadata_log_entries: usize,
    pub lance_max_versions: usize,
    pub lance_target_fragment_rows: u64,
    pub lance_small_fragment_count: usize,
    pub lance_deletion_compaction_threshold: f64,
    pub fail_on: Severity,
    pub only: BTreeSet<String>,
    pub disabled: BTreeSet<String>,
    pub declarative_rule_files: Vec<PathBuf>,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            min_row_group_rows: 100_000,
            small_file_bytes: 64 * 1024 * 1024,
            iceberg_max_snapshots: 100,
            iceberg_max_metadata_log_entries: 100,
            lance_max_versions: 100,
            lance_target_fragment_rows: 1_048_576,
            lance_small_fragment_count: 8,
            lance_deletion_compaction_threshold: 0.1,
            fail_on: Severity::Error,
            only: BTreeSet::new(),
            disabled: BTreeSet::new(),
            declarative_rule_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub format: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: "text".to_string(),
        }
    }
}
