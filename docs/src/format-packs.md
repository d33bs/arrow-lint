# Format Packs

ArrowLint includes scanners and built-in rules for the common Arrow interchange
formats. The project also defines rule-pack boundaries for adjacent Arrow-native
and Arrow-adjacent formats.

## Built In

- Parquet: row groups, statistics, encodings, compression, logical types.
- Arrow IPC and Feather: schema metadata, nullability, timestamp portability.
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
Iceberg manifest lists, manifests, delete files, and referenced data files remain
future scanner layers; ArrowLint currently validates table metadata JSON only.
For Lance, ArrowLint decodes the latest local table manifest and verifies
locally resolvable references. It does not decode schema fields, index sections,
data pages, deletion-vector contents, remote base paths, branches, or tags.
For Vortex, ArrowLint reads the fixed envelope, postscript, and an uncompressed,
unencrypted footer. It does not decode layout, dtype, statistics, array, or data
segments.
