# Format Packs

ArrowLint includes scanners and built-in rules for the common Arrow interchange
formats. The project also defines rule-pack boundaries for adjacent Arrow-native
and Arrow-adjacent formats.

## Built In

- Parquet: row groups, statistics, encodings, compression, logical types.
- Arrow IPC and Feather: schema metadata, nullability, timestamp portability.

## External Rule Packs

- Iceberg: partition evolution, manifests, snapshots, delete files, schema IDs.
- Vortex: encoding selection, chunk sizing, statistics, zero-copy behavior.
- Lance: fragment sizing, indexes, vector columns, schema evolution.
- DuckDB: Arrow export compatibility, nested types, timestamp semantics,
  Parquet writer settings.

Inputs from these format families are identified consistently so external rule
packs can attach deeper scanners and checks without changing the report model.
