from pathlib import Path

PROJECT_ROOT = Path(__file__).parents[1]
TEXT_ROOTS = (
    PROJECT_ROOT / ".github",
    PROJECT_ROOT / "crates",
    PROJECT_ROOT / "docs" / "src",
    PROJECT_ROOT / "src",
    PROJECT_ROOT / "tests",
    PROJECT_ROOT / "tools",
)
TOP_LEVEL_FILES = (
    PROJECT_ROOT / "CITATION.cff",
    PROJECT_ROOT / "CONTRIBUTING.md",
    PROJECT_ROOT / "Cargo.toml",
    PROJECT_ROOT / "README.md",
    PROJECT_ROOT / "pyproject.toml",
)
TEXT_SUFFIXES = {".cff", ".md", ".py", ".rs", ".toml", ".yaml", ".yml"}


def test_product_name_uses_lowercase_hyphenated_branding() -> None:
    forbidden_brand = "Arrow" + "Lint"
    files = list(TOP_LEVEL_FILES)
    for root in TEXT_ROOTS:
        files.extend(
            path
            for path in root.rglob("*")
            if path.is_file() and path.suffix in TEXT_SUFFIXES
        )

    offenders = [
        path.relative_to(PROJECT_ROOT)
        for path in files
        if forbidden_brand in path.read_text()
    ]

    assert offenders == []
