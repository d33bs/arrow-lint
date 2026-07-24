use std::path::PathBuf;

use arrowlint_core::{format_packs, lint_paths, list_builtin_rules, LintConfig, OutputFormat};
use pyo3::{exceptions::PyRuntimeError, prelude::*};

#[pyfunction]
fn lint_paths_json(paths: Vec<String>, config_path: Option<String>) -> PyResult<String> {
    let config = load_config(config_path)?;
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let report = lint_paths(&paths, config).map_err(to_py_error)?;
    serde_json::to_string_pretty(&report).map_err(to_py_error)
}

#[pyfunction]
fn render_lint(
    paths: Vec<String>,
    config_path: Option<String>,
    output_format: String,
) -> PyResult<String> {
    let config = load_config(config_path)?;
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

fn load_config(config_path: Option<String>) -> PyResult<LintConfig> {
    match config_path {
        Some(path) => LintConfig::from_path(path).map_err(to_py_error),
        None => Ok(LintConfig::default()),
    }
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
    Ok(())
}
