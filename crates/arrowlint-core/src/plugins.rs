use crate::{dataset::Dataset, diagnostics::Diagnostic, LintConfig};

pub trait Rule: Send + Sync {
    fn metadata(&self) -> crate::rules::RuleMetadata;
    fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic>;
}

#[derive(Default)]
pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<R>(&mut self, rule: R)
    where
        R: Rule + 'static,
    {
        self.rules.push(Box::new(rule));
    }

    pub fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic> {
        self.rules
            .iter()
            .filter(|rule| config.is_rule_enabled(rule.metadata().id))
            .flat_map(|rule| rule.check(dataset, config))
            .collect()
    }

    pub fn metadata(&self) -> Vec<crate::rules::RuleMetadata> {
        self.rules.iter().map(|rule| rule.metadata()).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{dataset::Dataset, diagnostics::Diagnostic, rules::RuleMetadata, Severity};

    use super::{Rule, RuleRegistry};

    struct MustNotRun;

    impl Rule for MustNotRun {
        fn metadata(&self) -> RuleMetadata {
            RuleMetadata {
                id: "TEST001",
                name: "must-not-run",
                category: "test",
                default_severity: Severity::Error,
                summary: "Fails when an excluded rule is executed.",
            }
        }

        fn check(&self, _dataset: &Dataset, _config: &crate::LintConfig) -> Vec<Diagnostic> {
            panic!("excluded rule executed")
        }
    }

    #[test]
    fn disabled_rules_are_not_executed() {
        let mut registry = RuleRegistry::new();
        registry.register(MustNotRun);
        let mut config = crate::LintConfig::default();
        config.rules.disabled.insert("TEST001".to_string());

        assert!(registry.check(&Dataset::default(), &config).is_empty());
    }

    #[test]
    fn rules_omitted_from_only_are_not_executed() {
        let mut registry = RuleRegistry::new();
        registry.register(MustNotRun);
        let mut config = crate::LintConfig::default();
        config.rules.only.insert("TEST002".to_string());

        assert!(registry.check(&Dataset::default(), &config).is_empty());
    }
}
