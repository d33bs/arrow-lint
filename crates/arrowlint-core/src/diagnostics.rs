use std::{cmp::Ordering, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    #[default]
    Warning,
    Error,
}

impl Severity {
    pub fn rank(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Error => 3,
        }
    }
}

impl Ord for Severity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl PartialOrd for Severity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule_id: String,
    pub severity: Severity,
    pub category: String,
    pub message: String,
    pub path: Option<String>,
    pub location: Option<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(
        rule_id: impl Into<String>,
        severity: Severity,
        category: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            severity,
            category: category.into(),
            message: message.into(),
            path: None,
            location: None,
            help: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}
