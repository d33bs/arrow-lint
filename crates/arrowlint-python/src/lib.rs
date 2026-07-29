use std::path::PathBuf;

use arrowlint_core::{
    diff_paths, format_packs, lint_paths, list_builtin_rules, LintConfig, OutputFormat,
};
use pyo3::{exceptions::PyRuntimeError, prelude::*};

#[pyfunction(signature = (paths, config_path, only_rules=None, disabled_rules=None))]
fn lint_paths_json(
    paths: Vec<String>,
    config_path: Option<String>,
    only_rules: Option<Vec<String>>,
    disabled_rules: Option<Vec<String>>,
) -> PyResult<String> {
    let config = load_config(config_path, only_rules, disabled_rules)?;
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let report = lint_paths(&paths, config).map_err(to_py_error)?;
    serde_json::to_string_pretty(&report).map_err(to_py_error)
}

#[pyfunction(signature = (
    paths,
    config_path,
    output_format,
    only_rules=None,
    disabled_rules=None
))]
fn render_lint(
    paths: Vec<String>,
    config_path: Option<String>,
    output_format: String,
    only_rules: Option<Vec<String>>,
    disabled_rules: Option<Vec<String>>,
) -> PyResult<String> {
    let config = load_config(config_path, only_rules, disabled_rules)?;
    let fail_format = OutputFormat::parse(&output_format);
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let report = lint_paths(&paths, config).map_err(to_py_error)?;
    report.render(fail_format).map_err(to_py_error)
}

#[pyfunction]
fn rules_json() -> PyResult<String> {
    serde_json::to_string_pretty(&list_builtin_rules()).map_err(to_py_error)
}

#[pyfunction]
fn formats_json() -> PyResult<String> {
    serde_json::to_string_pretty(&format_packs::known_format_packs()).map_err(to_py_error)
}

#[pyfunction]
fn diff_paths_json(old_path: String, new_path: String) -> PyResult<String> {
    let old_path = PathBuf::from(old_path);
    let new_path = PathBuf::from(new_path);
    let report = diff_paths(&old_path, &new_path).map_err(to_py_error)?;
    serde_json::to_string_pretty(&report).map_err(to_py_error)
}

#[pyfunction]
fn render_diff(old_path: String, new_path: String, output_format: String) -> PyResult<String> {
    let old_path = PathBuf::from(old_path);
    let new_path = PathBuf::from(new_path);
    let report = diff_paths(&old_path, &new_path).map_err(to_py_error)?;
    report
        .render(OutputFormat::parse(&output_format))
        .map_err(to_py_error)
}

fn load_config(
    config_path: Option<String>,
    only_rules: Option<Vec<String>>,
    disabled_rules: Option<Vec<String>>,
) -> PyResult<LintConfig> {
    let mut config = match config_path {
        Some(path) => LintConfig::from_path(path).map_err(to_py_error),
        None => Ok(LintConfig::default()),
    }?;
    if let Some(only_rules) = only_rules {
        config.rules.only = only_rules.into_iter().collect();
    }
    if let Some(disabled_rules) = disabled_rules {
        config.rules.disabled.extend(disabled_rules);
    }
    Ok(config)
}

fn to_py_error(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(lint_paths_json, module)?)?;
    module.add_function(wrap_pyfunction!(render_lint, module)?)?;
    module.add_function(wrap_pyfunction!(rules_json, module)?)?;
    module.add_function(wrap_pyfunction!(formats_json, module)?)?;
    module.add_function(wrap_pyfunction!(diff_paths_json, module)?)?;
    module.add_function(wrap_pyfunction!(render_diff, module)?)?;
    Ok(())
}
