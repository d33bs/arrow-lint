pub mod config;
pub mod dataset;
pub mod declarative;
pub mod diagnostics;
pub mod diff;
pub mod format_packs;
mod iceberg;
mod lance;
pub mod plugins;
pub mod report;
pub mod rules;
pub mod scanners;

pub use config::LintConfig;
pub use diagnostics::{Diagnostic, Severity};
pub use diff::{diff_paths, diff_paths_with_config, DiffReport};
pub use report::{LintReport, OutputFormat};

use std::path::PathBuf;

use anyhow::Result;

pub fn lint_paths(paths: &[PathBuf], config: LintConfig) -> Result<LintReport> {
    let dataset = scanners::scan_paths(paths, &config.scan)?;
    let registry = rules::builtin_registry();
    let mut diagnostics = registry.check(&dataset, &config);

    for rule in &config.declarative_rules {
        if config.is_rule_enabled(&rule.rule) {
            diagnostics.extend(declarative::evaluate(rule, &dataset));
        }
    }

    diagnostics.retain(|diagnostic| config.is_rule_enabled(&diagnostic.rule_id));
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.rule_id.cmp(&right.rule_id))
            .then(left.message.cmp(&right.message))
    });

    Ok(LintReport {
        dataset,
        diagnostics,
    })
}

pub fn list_builtin_rules() -> Vec<rules::RuleMetadata> {
    rules::builtin_registry().metadata()
}
