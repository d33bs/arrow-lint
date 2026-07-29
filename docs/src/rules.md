# Rules

ArrowLint ships with built-in rules for common Arrow, Parquet, Iceberg, Lance,
and Vortex quality issues. Rules classified as errors identify internally
inconsistent metadata. Warnings identify deprecated features or broadly
applicable interoperability and performance concerns. Informational diagnostics
identify review opportunities.

| Rule    | Severity | Category         | Purpose                                                 |
| ------- | -------- | ---------------- | ------------------------------------------------------- |
| `AL001` | warning  | performance      | flags tiny Parquet row groups                           |
| `AL002` | warning  | metadata         | flags Parquet column chunks missing statistics          |
| `AL003` | error    | schema           | flags schema drift across files in one target           |
| `AL004` | info     | metadata         | reports missing Arrow schema metadata                   |
| `AL005` | warning  | interoperability | flags timestamp choices that may not round-trip cleanly |
| `AL006` | warning  | encoding         | flags mixed dictionary encoding strategy                |
| `AL007` | info     | performance      | reports many small files                                |
| `AL008` | warning  | performance      | flags uncompressed Parquet columns                      |
| `AL009` | warning  | interoperability | flags the deprecated Parquet `INT96` physical type      |
| `AL010` | warning  | interoperability | flags deprecated Parquet encodings                      |
| `AL011` | warning  | interoperability | flags the deprecated Parquet `LZ4` codec                |
| `AL012` | error    | correctness      | flags impossible Parquet statistic counts               |
| `AL013` | error    | correctness      | flags inconsistent Parquet file and row-group counts    |
| `AL014` | warning  | metadata         | flags statistics that omit explicit null counts         |
| `AL015` | error    | correctness      | flags negative Parquet counts and byte sizes            |
| `AL100` | info     | extension        | identifies inputs handled by external format packs      |
| `AL101` | error    | iceberg          | flags missing or unsupported Iceberg format versions    |
| `AL102` | error    | iceberg          | flags invalid required Iceberg table metadata           |
| `AL103` | error    | iceberg          | flags broken current, default, and snapshot references  |
| `AL104` | error    | iceberg          | flags duplicate schema, spec, sort, and snapshot IDs    |
| `AL105` | error    | iceberg          | flags inconsistent snapshots and snapshot logs          |
| `AL106` | error    | iceberg          | flags invalid schema and partition field IDs            |
| `AL107` | info     | maintenance      | flags large snapshot and metadata histories             |
| `AL201` | error    | lance            | flags invalid or inconsistent Lance manifests           |
| `AL202` | error    | lance            | flags invalid Lance fragment identifiers                |
| `AL203` | error    | lance            | flags invalid Lance data-file field mappings            |
| `AL204` | error    | lance            | flags invalid Lance deletion metadata                   |
| `AL205` | error    | interoperability | flags unsupported or inconsistent Lance features        |
| `AL206` | error    | lance            | flags missing or inconsistent local Lance references    |
| `AL207` | info     | maintenance      | flags Lance compaction and retention opportunities      |
| `AL301` | error    | vortex           | flags invalid Vortex envelopes and postscripts          |
| `AL302` | error    | vortex           | flags invalid Vortex postscript segment locators        |
| `AL303` | error    | vortex           | flags invalid Vortex user metadata                      |
| `AL304` | error    | vortex           | flags invalid Vortex footer segment registries          |
| `AL305` | error    | vortex           | flags invalid Vortex array and layout registries        |
| `AL306` | error    | interoperability | flags incompatible Vortex compression metadata          |
| `AL307` | info     | performance      | reports missing Vortex optimization metadata            |

## Parquet Validity and Compatibility

### `AL009` — Deprecated Parquet Physical Types

Reports each Parquet column chunk whose physical type is `INT96`. The Parquet
format explicitly deprecates `INT96` and directs new writers not to emit it.
Rewrite timestamp columns as `INT64` with a `TIMESTAMP` logical annotation.

This rule is a warning because existing readers commonly support legacy
`INT96` files, even though new files should not use the type.

Reference: [Parquet physical types](https://parquet.apache.org/docs/file-format/types/)

### `AL010` — Deprecated Parquet Encodings

Reports column chunks whose encoding set contains:

- `PLAIN_DICTIONARY`, which has been superseded by `RLE_DICTIONARY` for data
  pages.
- `BIT_PACKED`, which has been superseded by the hybrid `RLE` encoding for
  repetition and definition levels.

The diagnostic identifies the row group, column path, and all deprecated
encodings found in that column chunk. Rewrite the file with a current Parquet
writer rather than changing reader configuration.

Reference: [Parquet encoding definitions](https://parquet.apache.org/docs/file-format/data-pages/encodings/)

### `AL011` — Deprecated Parquet Compression

Reports column chunks using the deprecated `LZ4` codec. `LZ4` and `LZ4_RAW` are
different Parquet codec identifiers: this rule does not report `LZ4_RAW`.
Rewrite affected columns with `LZ4_RAW`, `ZSTD`, or `SNAPPY`, based on the
required reader compatibility and workload.

Reference: [Parquet compression codecs](https://github.com/apache/parquet-format/blob/master/Compression.md)

### `AL012` — Invalid Parquet Statistics

Reports impossible count metadata as an error:

- `null_count` greater than the column chunk's `num_values`.
- `distinct_count` greater than the column chunk's `num_values`.

Readers use statistics for predicate pruning. Impossible counts indicate
corrupt or incorrectly produced metadata and should not be trusted. Rewrite
the file before using its statistics for query planning.

Reference: [Parquet `Statistics` definition](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL013` — Inconsistent Parquet Row Counts

Reports a file when `FileMetaData.num_rows` differs from the sum of
`RowGroup.num_rows`, or when that sum exceeds the signed 64-bit metadata range.
The rule does not run the comparison when a row group has a negative count;
`AL015` reports that more fundamental error instead.

This diagnostic is an error because both metadata locations describe the same
rows and must agree.

Reference: [Parquet metadata definition](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL014` — Missing Parquet Null Counts

Reports a column chunk that has a statistics structure but does not include
`null_count`. The Parquet specification recommends writing this field even
when the count is zero or the column is required. Readers must distinguish a
missing count from a known zero count.

This rule does not report column chunks with no statistics at all; `AL002`
handles that case.

Reference: [Parquet `Statistics.null_count`](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL015` — Invalid Parquet Size Metadata

Reports negative values in metadata fields that represent counts or sizes:

- File and row-group row counts.
- Row-group uncompressed byte sizes.
- Column-chunk value counts.
- Column-chunk compressed and uncompressed byte sizes.

Zero remains valid for empty files, empty row groups, and empty column chunks.
Negative values are structurally invalid and are reported as errors.

Reference: [Parquet metadata structures](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

## Iceberg Table Metadata

ArrowLint recognizes standard `*.metadata.json`, `*.gz.metadata.json`, and
`*.metadata.json.gz` files. The scanner parses table metadata once and applies
`AL101` through `AL107`. These rules follow the adopted Iceberg format versions
1 through 3. Version 4 remains under development and is not accepted until it is
formally adopted and implemented.

Reference: [Iceberg format versioning](https://iceberg.apache.org/spec/#format-versioning)

### `AL101` — Unsupported Iceberg Format Version

Reports a missing, non-integer, or unsupported `format-version`. Iceberg readers
must reject versions newer than they support. ArrowLint accepts the adopted
versions 1, 2, and 3.

### `AL102` — Invalid Required Iceberg Metadata

Validates version-specific required fields, including the table location,
update time, column ID high-water mark, schemas, partition specs, sort orders,
table UUID, sequence number, and v3 row-lineage high-water mark. Locations must
be absolute, and counters and timestamps must be non-negative.

Reference: [Iceberg table metadata fields](https://iceberg.apache.org/spec/#table-metadata-fields)

### `AL103` — Invalid Iceberg References

Checks that `current-schema-id`, `default-spec-id`,
`default-sort-order-id`, `current-snapshot-id`, and named snapshot references
point to entries retained in the same metadata file. When `refs` is present,
the `main` branch must match the current snapshot.

### `AL104` — Duplicate Iceberg Identifiers

Reports duplicate schema IDs, partition spec IDs, sort-order IDs, and snapshot
IDs. These identifiers are the stable links used by manifests, snapshots, and
writers; ambiguity can make table state unreadable.

### `AL105` — Invalid Iceberg Snapshots

Validates required snapshot fields by format version, sequence-number bounds,
manifest-list usage, summary operations, schema references, v3 row-lineage
fields, and snapshot-log ordering. The latest snapshot-log entry must match the
current snapshot, and log timestamps cannot be later than `last-updated-ms`.

Reference: [Iceberg snapshots](https://iceberg.apache.org/spec/#snapshots)

### `AL106` — Invalid Iceberg Field IDs

Recursively checks struct, list, and map field IDs for uniqueness and positive
values. It verifies that `last-column-id` and `last-partition-id` are not below
assigned IDs and that partition and sort fields reference known schema fields.

Reference: [Iceberg schema and partition evolution](https://iceberg.apache.org/spec/#partition-evolution)

### `AL107` — Iceberg Metadata Maintenance

Reports an informational diagnostic when retained snapshots or metadata-log
entries exceed configured limits. Defaults are 100 entries for each history:

```yaml
rules:
  iceberg_max_snapshots: 100
  iceberg_max_metadata_log_entries: 100
```

Set a threshold to `0` to disable that measurement. Retention requirements vary,
so this rule does not prescribe deletion; it prompts review of time-travel needs,
snapshot expiration, `write.metadata.previous-versions-max`, and
`write.metadata.delete-after-commit.enabled`.

Reference: [Iceberg maintenance recommendations](https://iceberg.apache.org/docs/latest/maintenance/)

## Iceberg Scope

The built-in Iceberg scanner validates table metadata JSON. It does not yet open
manifest lists or manifests, verify referenced files exist, detect duplicate
data-file paths, inspect delete-file strategy, or measure manifest and data-file
sizes. A clean metadata result therefore establishes internal table-metadata
consistency, not complete table health.

## Lance Table Metadata

ArrowLint recognizes local `*.lance` dataset directories as one lint target,
including when they are discovered below a parent directory. It supports both
Lance manifest naming schemes, selects the latest attached version, validates
the `LANC` footer envelope, and decodes the stable protobuf fields needed by
`AL201` through `AL207`.

Reference: [Lance table format](https://lance.org/format/table/)

### `AL201` — Invalid Lance Manifest

Reports missing `_versions` metadata, missing attached manifests, malformed
manifest filenames, mixed V1 and V2 naming schemes, corrupt footer envelopes,
protobuf decoding failures, zero versions, and disagreement between the
manifest filename and payload version.

Reference: [Lance storage layout](https://lance.org/format/table/layout/)

### `AL202` — Invalid Lance Fragment Identifiers

Reports duplicate fragment IDs and a `max_fragment_id` high-water mark below an
ID used by the current manifest. Fragment IDs are stable dataset-wide
identifiers and cannot be ambiguous or move backward.

### `AL203` — Invalid Lance Data Files

Validates that data-file paths are non-empty safe relative paths, the forbidden
field ID `-1` is not persisted, active field IDs do not overlap across files in
one fragment, and V2 field-to-column mappings have matching lengths and unique
non-negative column indices. The tombstone field ID `-2` remains valid.

### `AL204` — Invalid Lance Deletion Files

Reports unknown deletion-file types, deletion files based on a future dataset
version, and deleted-row counts greater than the fragment's physical row count.
Both sparse Arrow deletion arrays and dense Roaring bitmaps are accepted.

### `AL205` — Incompatible Lance Features

Reports unknown required reader or writer feature bits and feature flags that
do not match deletion files, table config, or base paths in the manifest. It
also warns about the deprecated V2-format flag, unstable data overlays, and
data format `2.3`, which the current Lance specification marks unstable.

References: [Lance table feature flags](https://lance.org/format/table/versioning/),
[Lance file versions](https://lance.org/format/file/versioning/)

### `AL206` — Missing Lance References

Checks local data, deletion, and transaction files referenced by the latest
manifest. Known data-file sizes must match the local object size. Base-path IDs
must be unique and every referenced ID must exist. Files using external base
paths are not opened because remote access and credentials are outside the
local scanner's scope.

### `AL207` — Lance Maintenance Opportunities

Reports informational diagnostics for retained-version history, collections of
small fragments, and fragments whose deleted-row ratio exceeds configured
thresholds:

```yaml
rules:
  lance_max_versions: 100
  lance_target_fragment_rows: 1048576
  lance_small_fragment_count: 8
  lance_deletion_compaction_threshold: 0.1
```

Set any threshold to `0` to disable that measurement. The default fragment size
and deletion ratio match Lance's compaction defaults. Version cleanup is never
prescribed automatically because it removes time-travel history; tagged
versions and the required retention window must be reviewed first.

References: [Lance table maintenance](https://lance.org/guide/read_and_write/),
[Lance performance guide](https://lance.org/guide/performance/)

## Lance Scope

The built-in Lance scanner validates the latest local table manifest and
locally resolvable references. It does not decode schema fields, index sections,
data pages, deletion-vector contents, branches, tags, remote base paths, or
historical manifest payloads. A clean result establishes current manifest
consistency for the inspected metadata; it does not prove every data value or
index entry is correct.

## Vortex File Metadata

ArrowLint recognizes local `*.vortex` and `*.vx` files. The scanner performs
bounded reads of the fixed file envelope, postscript, and footer. Footer
inspection is limited to 64 MiB so an untrusted locator cannot exhaust local or
CI memory. It implements the stable Vortex file metadata schema directly so
this support does not raise ArrowLint's Rust version requirement to that of the
latest Vortex crates.

Reference: [Vortex file format specification](https://docs.vortex.dev/specs/file-format)

### `AL301` — Invalid Vortex Container

Validates the leading and trailing `VTXF` magic bytes, file-format version,
postscript length, and postscript FlatBuffer structure. It reports truncated
files, unsupported versions, offsets that place the postscript before the file
body, and malformed postscript metadata.

### `AL302` — Invalid Vortex Postscript Segments

Requires the layout and footer segments and validates every dtype, layout,
statistics, and footer locator that is present. Segment ranges must end before
the postscript, offsets must not overlap the leading magic, and offsets must
satisfy their declared power-of-two alignment.

### `AL303` — Invalid Vortex User Metadata

Validates the postscript's user-defined metadata. A file may contain at most 16
entries. Every entry must have a segment and a unique, non-empty UTF-8 key no
longer than 64 bytes. Metadata segment ranges and alignment are also checked.

### `AL304` — Invalid Vortex Footer Segments

Validates the footer FlatBuffer and its required segment registry. Segment
offsets must be ordered, ranges must remain inside the file, and offsets must
satisfy their declared alignment. These checks protect lazy reads and the
aligned buffers used by Vortex's zero-copy design. Footers over the inspection
limit receive an informational skip diagnostic rather than an error.

Reference: [Vortex serialization internals](https://docs.vortex.dev/developer-guide/internals/serialization)

### `AL305` — Invalid Vortex Registries

Validates the footer's array and layout registries. Every entry must contain a
non-empty identifier, and an identifier may not appear more than once in the
same registry.

### `AL306` — Incompatible Vortex Compression

Validates postscript and footer compression specifications. The compression
registry may contain at most eight entries, and the current stable schemes are
None, LZ4, ZLib, and ZStd. When the footer itself is compressed or encrypted,
ArrowLint reports that deep footer checks were skipped rather than treating
opaque bytes as malformed metadata.

### `AL307` — Missing Vortex Optimization Metadata

Reports independent informational diagnostics when a file omits its top-level
dtype or file-level statistics. An omitted dtype requires the reader to obtain
it externally. File-level statistics enable whole-file pruning; their value is
workload-dependent, so absence is informational rather than invalid.

Reference: [Vortex file-format concepts](https://docs.vortex.dev/concepts/file-format)

## Vortex Scope

The built-in Vortex scanner validates stable, local container metadata. It does
not decode dtype, layout, statistics, array, or data segments; verify registry
identifiers against a particular Vortex session; infer editions; or inspect
custom encodings. Compressed or encrypted footers remain opaque. A clean result
establishes structural consistency for the inspected metadata, not complete
readability or optimal encoding and layout choices.

## Parquet Rule Scope

The Parquet validity checks are metadata-only. They do not decode data pages or
compare stored statistics with every physical value. A clean result therefore
means the inspected footer metadata is internally consistent; it does not prove
that the complete file contents are uncorrupted.

## Selecting Rules

Run a focused rule set with repeatable CLI options:

```bash
arrow-lint lint dataset --only AL011 --only AL014
arrow-lint lint dataset --disable AL004
```

The native Rust CLI supports the same options:

```bash
cargo run -p arrowlint-cli -- lint dataset --only AL011
```

Persist selection in `.arrowlint.yaml` when a project needs the same policy on
every run:

```yaml
rules:
  only:
    - AL011
    - AL014
  disabled:
    - AL014
```

An empty or omitted `only` list enables all built-in and declarative rules. A
non-empty `only` list includes only matching rule IDs. `disabled` always takes
precedence, so the example runs `AL011` and suppresses `AL014`. Supplying
`--only` replaces the configured `only` list for that invocation; `--disable`
adds to the configured disabled rules.

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
