"""Command line interface for ArrowLint."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from arrow_lint.api import formats, lint, render, rules


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

    parser.error(f"unknown command: {args.command}")
    return 2


def trigger() -> None:
    raise SystemExit(main())


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
