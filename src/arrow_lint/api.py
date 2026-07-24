"""Python API for ArrowLint."""

from __future__ import annotations

import json
from collections.abc import Sequence
from pathlib import Path
from typing import Any, Literal

OutputFormat = Literal["text", "json", "sarif"]


def lint(
    paths: str | Path | Sequence[str | Path], config: str | Path | None = None
) -> dict[str, Any]:
    """Lint one or more Arrow dataset paths and return a JSON-compatible report."""

    from arrow_lint import _native

    normalized_paths = _normalize_paths(paths)
    config_path = str(config) if config is not None else None
    return json.loads(_native.lint_paths_json(normalized_paths, config_path))


def render(
    paths: str | Path | Sequence[str | Path],
    config: str | Path | None = None,
    output: OutputFormat = "text",
) -> str:
    """Lint paths and render the report as text, JSON, or SARIF."""

    from arrow_lint import _native

    normalized_paths = _normalize_paths(paths)
    config_path = str(config) if config is not None else None
    return _native.render_lint(normalized_paths, config_path, output)


def rules() -> list[dict[str, Any]]:
    """Return metadata for built-in ArrowLint rules."""

    from arrow_lint import _native

    return json.loads(_native.rules_json())


def formats() -> list[dict[str, Any]]:
    """Return known format packs and extension targets."""

    from arrow_lint import _native

    return json.loads(_native.formats_json())


def _normalize_paths(paths: str | Path | Sequence[str | Path]) -> list[str]:
    if isinstance(paths, str | Path):
        return [str(paths)]
    return [str(path) for path in paths]
