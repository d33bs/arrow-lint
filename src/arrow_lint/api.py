"""Python API for arrow-lint."""

from __future__ import annotations

import json
from collections.abc import Sequence
from pathlib import Path
from typing import Any, Literal

OutputFormat = Literal["text", "json", "sarif"]
DiffOutputFormat = Literal["text", "json"]


def lint(
    paths: str | Path | Sequence[str | Path],
    config: str | Path | None = None,
    *,
    only: Sequence[str] | None = None,
    disabled: Sequence[str] | None = None,
) -> dict[str, Any]:
    """Lint paths, optionally selecting or disabling rule IDs."""

    from arrow_lint import _native

    normalized_paths = _normalize_paths(paths)
    config_path = str(config) if config is not None else None
    only_rules = list(only) if only is not None else None
    disabled_rules = list(disabled) if disabled is not None else None
    return json.loads(
        _native.lint_paths_json(
            normalized_paths,
            config_path,
            only_rules,
            disabled_rules,
        )
    )


def render(
    paths: str | Path | Sequence[str | Path],
    config: str | Path | None = None,
    output: OutputFormat = "text",
    *,
    only: Sequence[str] | None = None,
    disabled: Sequence[str] | None = None,
) -> str:
    """Lint selected rules and render the report as text, JSON, or SARIF."""

    from arrow_lint import _native

    normalized_paths = _normalize_paths(paths)
    config_path = str(config) if config is not None else None
    only_rules = list(only) if only is not None else None
    disabled_rules = list(disabled) if disabled is not None else None
    return _native.render_lint(
        normalized_paths,
        config_path,
        output,
        only_rules,
        disabled_rules,
    )


def rules() -> list[dict[str, Any]]:
    """Return metadata for built-in arrow-lint rules."""

    from arrow_lint import _native

    return json.loads(_native.rules_json())


def formats() -> list[dict[str, Any]]:
    """Return known format packs and extension targets."""

    from arrow_lint import _native

    return json.loads(_native.formats_json())


def diff(old: str | Path, new: str | Path) -> dict[str, Any]:
    """Compare two Arrow dataset paths using metadata and column statistics."""

    from arrow_lint import _native

    return json.loads(_native.diff_paths_json(str(old), str(new)))


def render_diff(
    old: str | Path,
    new: str | Path,
    output: DiffOutputFormat = "text",
) -> str:
    """Compare two Arrow dataset paths and render text or JSON output."""

    from arrow_lint import _native

    return _native.render_diff(str(old), str(new), output)


def _normalize_paths(paths: str | Path | Sequence[str | Path]) -> list[str]:
    if isinstance(paths, str | Path):
        return [str(paths)]
    return [str(path) for path in paths]
