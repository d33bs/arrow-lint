"""Command line interface for ArrowLint."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from arrow_lint.api import diff, formats, lint, render, render_diff, rules


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="arrow-lint",
        description="Lint Apache Arrow datasets and related formats.",
    )
    subcommands = parser.add_subparsers(dest="command", required=True)

    lint_parser = subcommands.add_parser("lint", help="lint Arrow dataset files")
    lint_parser.add_argument("paths", nargs="+", type=Path)
    lint_parser.add_argument("--config", type=Path)
    lint_parser.add_argument(
        "--output",
        choices=("text", "json", "sarif"),
        default="text",
    )
    lint_parser.add_argument(
        "--fail-on",
        choices=("info", "warning", "error", "never"),
        default=None,
        help="override the config failure threshold for this invocation",
    )

    subcommands.add_parser("rules", help="list built-in rules")
    subcommands.add_parser("formats", help="list known format packs")
    _add_diff_arguments(
        subcommands.add_parser(
            "diff",
            help="compare Arrow datasets using metadata and statistics",
        ),
        include_exit_code=True,
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "lint":
        report = lint(args.paths, config=args.config)
        if args.output == "text":
            sys.stdout.write(render(args.paths, config=args.config, output="text"))
        elif args.output == "json":
            print(json.dumps(report, indent=2))
        else:
            sys.stdout.write(render(args.paths, config=args.config, output="sarif"))
        return _exit_code(report, args.fail_on)

    if args.command == "rules":
        for rule in rules():
            print(
                f"{rule['id']} {rule['name']:<30} "
                f"{rule['category']:<16} {rule['summary']}"
            )
        return 0

    if args.command == "formats":
        for format_pack in formats():
            focus = ", ".join(format_pack["best_practice_focus"])
            print(
                f"{format_pack['name']:<10} {format_pack['status']:<16} "
                f"{format_pack['rule_pack']:<20} {focus}"
            )
        return 0

    if args.command == "diff":
        return _run_diff(args)

    parser.error(f"unknown command: {args.command}")
    return 2


def trigger() -> None:
    raise SystemExit(main())


def diff_main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="arrowdiff",
        description="Git-style metadata and statistics diff for Arrow datasets.",
    )
    _add_diff_arguments(parser, include_exit_code=True)
    return _run_diff(parser.parse_args(argv))


def trigger_diff() -> None:
    raise SystemExit(diff_main())


def _exit_code(report: dict[str, object], fail_on: str | None) -> int:
    threshold = fail_on or "error"
    if threshold == "never":
        return 0
    ranks = {"info": 1, "warning": 2, "error": 3}
    minimum = ranks[threshold]
    diagnostics = report.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        return 1
    for diagnostic in diagnostics:
        if not isinstance(diagnostic, dict):
            continue
        severity = diagnostic.get("severity")
        if isinstance(severity, str) and ranks.get(severity, 0) >= minimum:
            return 1
    return 0


def _add_diff_arguments(
    parser: argparse.ArgumentParser, *, include_exit_code: bool
) -> None:
    parser.add_argument("old", type=Path)
    parser.add_argument("new", type=Path)
    parser.add_argument("--output", choices=("text", "json"), default="text")
    if include_exit_code:
        parser.add_argument(
            "--exit-code",
            action="store_true",
            help="return 1 when the datasets differ",
        )


def _run_diff(args: argparse.Namespace) -> int:
    report: dict[str, object] | None = None
    if args.output == "json":
        report = diff(args.old, args.new)
        print(json.dumps(report, indent=2))
    else:
        sys.stdout.write(render_diff(args.old, args.new, output="text"))

    if not args.exit_code:
        return 0
    if report is None:
        report = diff(args.old, args.new)
    return 1 if _diff_has_changes(report) else 0


def _diff_has_changes(report: dict[str, object]) -> bool:
    has_changes = report.get("has_changes")
    if isinstance(has_changes, bool):
        return has_changes

    schema = report.get("schema")
    if isinstance(schema, dict) and schema.get("identical") is False:
        return True

    columns = report.get("columns")
    if isinstance(columns, dict) and columns.get("changed"):
        return True

    metadata = report.get("metadata")
    if isinstance(metadata, dict) and any(
        metadata.get(change) for change in ("added", "removed", "changed")
    ):
        return True

    statistics = report.get("statistics")
    return isinstance(statistics, dict) and any(
        statistics.get(change)
        for change in (
            "file_count_changed",
            "row_count_changed",
            "row_groups_changed",
            "compression_changed",
            "scan_cost_changed",
        )
    )
