# Rules

ArrowLint ships with built-in rules for common Arrow and Parquet quality issues.

| Rule    | Category         | Purpose                                                 |
| ------- | ---------------- | ------------------------------------------------------- |
| `AL001` | performance      | flags tiny Parquet row groups                           |
| `AL002` | metadata         | flags Parquet column chunks missing statistics          |
| `AL003` | schema           | flags schema drift across files in one target           |
| `AL004` | metadata         | reports missing Arrow schema metadata                   |
| `AL005` | interoperability | flags timestamp choices that may not round-trip cleanly |
| `AL006` | encoding         | flags mixed dictionary encoding strategy                |
| `AL007` | performance      | reports many small files                                |
| `AL008` | performance      | flags uncompressed Parquet columns                      |
| `AL100` | extension        | identifies inputs handled by external format packs      |

## Declarative Rules

Use declarative YAML rules for simple metadata checks:

```yaml
rule: missing_crs
description: GeoParquet file is missing CRS metadata
severity: warning
applies_to: parquet
check:
  metadata_key: crs
```

Reference rule files from `.arrowlint.yaml`:

```yaml
rules:
  declarative_rule_files:
    - examples/rules/metadata.yaml
```

Declarative rules are best suited to metadata presence checks. Checks that need
record batch inspection, cross-file state, or format-specific metadata should be
implemented as Rust rules or dedicated rule packs.
