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
            .flat_map(|rule| rule.check(dataset, config))
            .collect()
    }

    pub fn metadata(&self) -> Vec<crate::rules::RuleMetadata> {
        self.rules.iter().map(|rule| rule.metadata()).collect()
    }
}
