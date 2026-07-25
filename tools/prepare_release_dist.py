"""Flatten downloaded release artifacts into a publishable dist directory."""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path

SUPPORTED_ARCHIVE_SUFFIXES = (".whl", ".tar.gz")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()

    prepare_release_dist(args.source, args.destination)
    return 0


def prepare_release_dist(source: Path, destination: Path) -> list[Path]:
    if not source.is_dir():
        raise SystemExit(f"release artifact source does not exist: {source}")
    destination.mkdir(parents=True, exist_ok=True)

    prepared: list[Path] = []
    seen_names: set[str] = set()
    for artifact in sorted(path for path in source.rglob("*") if path.is_file()):
        if not artifact.name.endswith(SUPPORTED_ARCHIVE_SUFFIXES):
            raise SystemExit(f"unexpected downloaded artifact: {artifact}")
        if artifact.name in seen_names:
            raise SystemExit(f"duplicate release artifact filename: {artifact.name}")
        seen_names.add(artifact.name)
        target = destination / artifact.name
        if target.exists():
            raise SystemExit(f"release artifact target already exists: {target}")
        shutil.copy2(artifact, target)
        prepared.append(target)

    if not prepared:
        raise SystemExit(f"no release artifacts found in {source}")

    for artifact in prepared:
        print(artifact.name)
    return prepared


if __name__ == "__main__":
    raise SystemExit(main())
