# Format Packs

arrow-lint includes scanners and built-in rules for the common Arrow interchange
formats. The project also defines rule-pack boundaries for adjacent Arrow-native
and Arrow-adjacent formats.

## Built In

- Parquet and GeoParquet: row groups, statistics, encodings, compression,
  logical types, geometry metadata, and spatial pruning metadata.
- Arrow IPC and Feather: file and stream framing, schema metadata, nullability,
  timestamp portability, and dictionary compatibility.
- Iceberg table metadata: format versions, required fields, schema and partition
  IDs, snapshot references, snapshot logs, and metadata history maintenance.
- Lance table metadata: manifest envelopes and versions, fragment and data-file
  invariants, feature flags, local references, deletion health, and maintenance.
- Vortex file metadata: container and postscript integrity, segment bounds and
  alignment, footer registries, compression compatibility, and optimization
  metadata.

## External Rule Packs

- DuckDB: Arrow export compatibility, nested types, timestamp semantics,
  Parquet writer settings.

Inputs from these format families are identified consistently so external rule
packs can attach deeper scanners and checks without changing the report model.
Single-file remote URLs are supported for Parquet, Arrow IPC, Feather, Iceberg
table metadata, and Vortex inputs, including HTTP(S), S3, GCS, and Azure object
store URIs. Authenticated object-store reads use standard provider environment
variables, and anonymous public object reads can use
`ARROW_LINT_OBJECT_STORE_ANONYMOUS=true` or provider-specific
`*_SKIP_SIGNATURE=true` variables. Remote URLs are downloaded into memory before
scanning; directory expansion and Lance dataset discovery remain local-only.
Iceberg manifest lists, manifests, delete files, and referenced data files remain
future scanner layers; arrow-lint currently validates table metadata JSON only.
For Lance, arrow-lint decodes the latest local table manifest and verifies
locally resolvable references. It does not decode schema fields, index sections,
data pages, deletion-vector contents, remote base paths, branches, or tags.
For Vortex, arrow-lint reads the fixed envelope, postscript, and an uncompressed,
unencrypted footer. It does not decode layout, dtype, statistics, array, or data
segments.
