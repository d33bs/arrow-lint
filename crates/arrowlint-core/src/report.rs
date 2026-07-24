use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{dataset::Dataset, diagnostics::Diagnostic, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    Sarif,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Self {
        match value {
            "json" => Self::Json,
            "sarif" => Self::Sarif,
            _ => Self::Text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintReport {
    pub dataset: Dataset,
    pub diagnostics: Vec<Diagnostic>,
}

impl LintReport {
    pub fn has_failure_at(&self, fail_on: Severity) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity >= fail_on)
    }

    pub fn render(&self, format: OutputFormat) -> anyhow::Result<String> {
        match format {
            OutputFormat::Text => Ok(self.render_text()),
            OutputFormat::Json => Ok(serde_json::to_string_pretty(self)?),
            OutputFormat::Sarif => Ok(serde_json::to_string_pretty(&self.render_sarif())?),
        }
    }

    fn render_text(&self) -> String {
        if self.diagnostics.is_empty() {
            return format!(
                "ArrowLint checked {} file(s): no issues found\n",
                self.dataset.files.len()
            );
        }

        let mut lines = vec![format!(
            "ArrowLint checked {} file(s): {} issue(s)",
            self.dataset.files.len(),
            self.diagnostics.len()
        )];
        for diagnostic in &self.diagnostics {
            let path = diagnostic.path.as_deref().unwrap_or("<dataset>");
            let location = diagnostic
                .location
                .as_deref()
                .map(|value| format!(":{value}"))
                .unwrap_or_default();
            lines.push(format!(
                "{}{}: {} {} {}",
                path, location, diagnostic.severity, diagnostic.rule_id, diagnostic.message
            ));
            if let Some(help) = &diagnostic.help {
                lines.push(format!("  help: {help}"));
            }
        }
        lines.push(String::new());
        lines.join("\n")
    }

    fn render_sarif(&self) -> serde_json::Value {
        let results = self
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let uri = diagnostic.path.as_deref().unwrap_or("dataset");
                json!({
                    "ruleId": diagnostic.rule_id,
                    "level": sarif_level(diagnostic.severity),
                    "message": { "text": diagnostic.message },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": uri }
                        }
                    }]
                })
            })
            .collect::<Vec<_>>();

        json!({
            "version": "2.1.0",
            "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "ArrowLint",
                        "informationUri": "https://github.com/CU-DBMI/arrow-lint"
                    }
                },
                "results": results
            }]
        })
    }
}

fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "note",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}
