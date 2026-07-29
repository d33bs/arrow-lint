# Format Packs

ArrowLint includes scanners and built-in rules for the common Arrow interchange
formats. The project also defines rule-pack boundaries for adjacent Arrow-native
and Arrow-adjacent formats.

## Built In

- Parquet: row groups, statistics, encodings, compression, logical types.
- Arrow IPC and Feather: schema metadata, nullability, timestamp portability.
- Iceberg table metadata: format versions, required fields, schema and partition
  IDs, snapshot references, snapshot logs, and metadata history maintenance.

## External Rule Packs

- Vortex: encoding selection, chunk sizing, statistics, zero-copy behavior.
- Lance: fragment sizing, indexes, vector columns, schema evolution.
- DuckDB: Arrow export compatibility, nested types, timestamp semantics,
  Parquet writer settings.

Inputs from these format families are identified consistently so external rule
packs can attach deeper scanners and checks without changing the report model.
Iceberg manifest lists, manifests, delete files, and referenced data files remain
future scanner layers; ArrowLint currently validates table metadata JSON only.
