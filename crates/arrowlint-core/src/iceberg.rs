use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::{
    dataset::{Dataset, DatasetFile, Format},
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    rules::RuleMetadata,
    LintConfig,
};

pub(crate) fn register_rules(registry: &mut RuleRegistry) {
    registry.register(IcebergFormatVersion);
    registry.register(IcebergRequiredMetadata);
    registry.register(IcebergReferences);
    registry.register(IcebergIdentifiers);
    registry.register(IcebergSnapshots);
    registry.register(IcebergFieldIds);
    registry.register(IcebergMetadataMaintenance);
}

struct IcebergFormatVersion;

impl Rule for IcebergFormatVersion {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL101",
            name: "unsupported-iceberg-format-version",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg metadata must use an adopted format version supported by arrow-lint.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        iceberg_objects(dataset)
            .filter_map(|(file, metadata)| match integer(metadata, "format-version") {
                Some(1..=3) => None,
                Some(version) => Some(
                    iceberg_diagnostic(
                        "AL101",
                        Severity::Error,
                        file,
                        format!("unsupported Iceberg format version {version}"),
                    )
                    .with_help(
                        "use an adopted Iceberg format version from 1 through 3, or upgrade arrow-lint when a newer version is adopted",
                    ),
                ),
                None => Some(
                    iceberg_diagnostic(
                        "AL101",
                        Severity::Error,
                        file,
                        "`format-version` is missing or is not an integer",
                    )
                    .with_help("write a valid Iceberg table metadata JSON file"),
                ),
            })
            .collect()
    }
}

struct IcebergRequiredMetadata;

impl Rule for IcebergRequiredMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL102",
            name: "invalid-iceberg-required-metadata",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg table metadata must contain version-appropriate required fields.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            let Some(version @ 1..=3) = integer(metadata, "format-version") else {
                continue;
            };

            require_non_empty_string(
                &mut diagnostics,
                file,
                metadata,
                "location",
                "table location",
            );
            if let Some(location) = string(metadata, "location") {
                if !is_absolute_location(location) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL102",
                            Severity::Error,
                            file,
                            "`location` must be an absolute path or URI",
                        )
                        .with_location("field=location"),
                    );
                }
            }
            require_non_negative_integer(&mut diagnostics, file, metadata, "last-updated-ms");
            require_non_negative_integer(&mut diagnostics, file, metadata, "last-column-id");

            if version == 1 {
                if !has_object(metadata, "schema")
                    && !(has_non_empty_array(metadata, "schemas")
                        && integer(metadata, "current-schema-id").is_some())
                {
                    diagnostics.push(missing_required(file, "schema"));
                }
                if !has_array(metadata, "partition-spec")
                    && !(has_non_empty_array(metadata, "partition-specs")
                        && integer(metadata, "default-spec-id").is_some())
                {
                    diagnostics.push(missing_required(file, "partition-spec"));
                }
                continue;
            }

            require_uuid(&mut diagnostics, file, metadata);
            require_non_negative_integer(&mut diagnostics, file, metadata, "last-sequence-number");
            require_non_empty_array(&mut diagnostics, file, metadata, "schemas");
            require_integer(&mut diagnostics, file, metadata, "current-schema-id");
            require_non_empty_array(&mut diagnostics, file, metadata, "partition-specs");
            require_integer(&mut diagnostics, file, metadata, "default-spec-id");
            require_non_negative_integer(&mut diagnostics, file, metadata, "last-partition-id");
            require_array(&mut diagnostics, file, metadata, "sort-orders");
            require_integer(&mut diagnostics, file, metadata, "default-sort-order-id");
            if version == 3 {
                require_non_negative_integer(&mut diagnostics, file, metadata, "next-row-id");
            }
        }
        diagnostics
    }
}

struct IcebergReferences;

impl Rule for IcebergReferences {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL103",
            name: "invalid-iceberg-references",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg current and default IDs must reference metadata entries.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            check_reference(
                &mut diagnostics,
                file,
                metadata,
                "current-schema-id",
                "schemas",
                "schema-id",
            );
            check_reference(
                &mut diagnostics,
                file,
                metadata,
                "default-spec-id",
                "partition-specs",
                "spec-id",
            );
            if integer(metadata, "default-sort-order-id") != Some(0) {
                check_reference(
                    &mut diagnostics,
                    file,
                    metadata,
                    "default-sort-order-id",
                    "sort-orders",
                    "order-id",
                );
            }

            let snapshot_ids = object_ids(metadata, "snapshots", "snapshot-id");
            let current_snapshot_id = current_snapshot_id(metadata);
            if let Some(current_snapshot_id) = current_snapshot_id {
                if !snapshot_ids.contains(&current_snapshot_id) {
                    diagnostics.push(invalid_reference(
                        file,
                        "current-snapshot-id",
                        current_snapshot_id,
                        "snapshots",
                    ));
                }
            }

            if let Some(refs) = metadata.get("refs").and_then(Value::as_object) {
                if let Some(main) = refs.get("main").and_then(Value::as_object) {
                    let main_snapshot_id = integer(main, "snapshot-id");
                    if main_snapshot_id != current_snapshot_id {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL103",
                                Severity::Error,
                                file,
                                "`refs.main.snapshot-id` does not match `current-snapshot-id`",
                            )
                            .with_location("field=refs.main"),
                        );
                    }
                } else {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL103",
                            Severity::Error,
                            file,
                            "`refs` is present but has no `main` branch",
                        )
                        .with_location("field=refs"),
                    );
                }

                for (name, reference) in refs {
                    let Some(snapshot_id) = reference
                        .as_object()
                        .and_then(|reference| integer(reference, "snapshot-id"))
                    else {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL103",
                                Severity::Error,
                                file,
                                format!("snapshot reference `{name}` has no integer `snapshot-id`"),
                            )
                            .with_location(format!("field=refs.{name}")),
                        );
                        continue;
                    };
                    if !snapshot_ids.contains(&snapshot_id) {
                        diagnostics.push(invalid_reference(
                            file,
                            &format!("refs.{name}.snapshot-id"),
                            snapshot_id,
                            "snapshots",
                        ));
                    }
                }
            }
        }
        diagnostics
    }
}

struct IcebergIdentifiers;

impl Rule for IcebergIdentifiers {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL104",
            name: "duplicate-iceberg-identifiers",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg schemas, specs, sort orders, and snapshots must have unique IDs.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            for (array_key, id_key) in [
                ("schemas", "schema-id"),
                ("partition-specs", "spec-id"),
                ("sort-orders", "order-id"),
                ("snapshots", "snapshot-id"),
            ] {
                for duplicate in duplicate_object_ids(metadata, array_key, id_key) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL104",
                            Severity::Error,
                            file,
                            format!("duplicate `{id_key}` value {duplicate} in `{array_key}`"),
                        )
                        .with_location(format!("field={array_key}")),
                    );
                }
            }
        }
        diagnostics
    }
}

struct IcebergSnapshots;

impl Rule for IcebergSnapshots {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL105",
            name: "invalid-iceberg-snapshots",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg snapshots and snapshot logs must be internally consistent.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            let Some(version @ 1..=3) = integer(metadata, "format-version") else {
                continue;
            };
            let schema_ids = object_ids(metadata, "schemas", "schema-id");
            let snapshot_ids = object_ids(metadata, "snapshots", "snapshot-id");
            let last_sequence_number = integer(metadata, "last-sequence-number");
            let last_updated_ms = integer(metadata, "last-updated-ms");

            for (index, snapshot) in objects(metadata, "snapshots").enumerate() {
                let location = format!("field=snapshots[{index}]");
                let snapshot_id = integer(snapshot, "snapshot-id");
                if snapshot_id.is_none_or(|snapshot_id| snapshot_id <= 0) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL105",
                            Severity::Error,
                            file,
                            "snapshot has no positive integer `snapshot-id`",
                        )
                        .with_location(location.clone()),
                    );
                }
                let timestamp_ms = integer(snapshot, "timestamp-ms");
                if timestamp_ms.is_none_or(|timestamp_ms| timestamp_ms < 0) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL105",
                            Severity::Error,
                            file,
                            "snapshot has no non-negative integer `timestamp-ms`",
                        )
                        .with_location(location.clone()),
                    );
                } else if timestamp_ms
                    .zip(last_updated_ms)
                    .is_some_and(|(timestamp_ms, last_updated_ms)| timestamp_ms > last_updated_ms)
                {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL105",
                            Severity::Error,
                            file,
                            "snapshot timestamp is later than `last-updated-ms`",
                        )
                        .with_location(location.clone()),
                    );
                }

                if snapshot.contains_key("manifest-list") && snapshot.contains_key("manifests") {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL105",
                            Severity::Error,
                            file,
                            "snapshot contains both `manifest-list` and deprecated `manifests`",
                        )
                        .with_location(location.clone()),
                    );
                }

                if version >= 2 {
                    let sequence_number = integer(snapshot, "sequence-number");
                    if sequence_number.is_none_or(|sequence_number| sequence_number < 0) {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL105",
                                Severity::Error,
                                file,
                                "v2+ snapshot has no non-negative `sequence-number`",
                            )
                            .with_location(location.clone()),
                        );
                    } else if sequence_number.zip(last_sequence_number).is_some_and(
                        |(sequence_number, last_sequence_number)| {
                            sequence_number > last_sequence_number
                        },
                    ) {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL105",
                                Severity::Error,
                                file,
                                "snapshot sequence exceeds `last-sequence-number`",
                            )
                            .with_location(location.clone()),
                        );
                    }
                    if string(snapshot, "manifest-list").is_none_or(str::is_empty) {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL105",
                                Severity::Error,
                                file,
                                "v2+ snapshot has no non-empty `manifest-list`",
                            )
                            .with_location(location.clone()),
                        );
                    }
                    let operation = snapshot
                        .get("summary")
                        .and_then(Value::as_object)
                        .and_then(|summary| string(summary, "operation"));
                    if !matches!(
                        operation,
                        Some("append" | "replace" | "overwrite" | "delete")
                    ) {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL105",
                                Severity::Error,
                                file,
                                "v2+ snapshot summary has no recognized `operation`",
                            )
                            .with_location(location.clone()),
                        );
                    }
                }

                if let Some(schema_id) = integer(snapshot, "schema-id") {
                    if !schema_ids.contains(&schema_id) {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL105",
                                Severity::Error,
                                file,
                                format!("snapshot references unknown schema ID {schema_id}"),
                            )
                            .with_location(location.clone()),
                        );
                    }
                }

                if version == 3 {
                    for field in ["first-row-id", "added-rows"] {
                        if integer(snapshot, field).is_none_or(|value| value < 0) {
                            diagnostics.push(
                                iceberg_diagnostic(
                                    "AL105",
                                    Severity::Error,
                                    file,
                                    format!("v3 snapshot has no non-negative `{field}`"),
                                )
                                .with_location(location.clone()),
                            );
                        }
                    }
                }
            }

            check_snapshot_log(
                &mut diagnostics,
                file,
                metadata,
                &snapshot_ids,
                last_updated_ms,
            );
        }
        diagnostics
    }
}

struct IcebergFieldIds;

impl Rule for IcebergFieldIds {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL106",
            name: "invalid-iceberg-field-ids",
            category: "iceberg",
            default_severity: Severity::Error,
            summary: "Iceberg field IDs must be valid and preserve evolution invariants.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            let version = integer(metadata, "format-version").unwrap_or(1);
            let mut all_field_ids = BTreeSet::new();
            let mut highest_field_id = None;
            for (index, schema) in schemas(metadata).into_iter().enumerate() {
                let id_values = schema_field_id_values(schema);
                if id_values.iter().any(Option::is_none) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL106",
                            Severity::Error,
                            file,
                            "schema contains a field, list element, or map key/value without an integer ID",
                        )
                        .with_location(format!("field=schemas[{index}]")),
                    );
                }
                let ids = id_values.into_iter().flatten().collect::<Vec<_>>();
                for duplicate in duplicates(&ids) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL106",
                            Severity::Error,
                            file,
                            format!("schema contains duplicate field ID {duplicate}"),
                        )
                        .with_location(format!("field=schemas[{index}]")),
                    );
                }
                for id in ids {
                    if id <= 0 {
                        diagnostics.push(
                            iceberg_diagnostic(
                                "AL106",
                                Severity::Error,
                                file,
                                format!("schema contains non-positive field ID {id}"),
                            )
                            .with_location(format!("field=schemas[{index}]")),
                        );
                    }
                    highest_field_id =
                        Some(highest_field_id.map_or(id, |highest: i64| highest.max(id)));
                    all_field_ids.insert(id);
                }
            }
            if let (Some(last_column_id), Some(highest_field_id)) =
                (integer(metadata, "last-column-id"), highest_field_id)
            {
                if last_column_id < highest_field_id {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL106",
                            Severity::Error,
                            file,
                            format!(
                                "`last-column-id` {last_column_id} is below assigned field ID {highest_field_id}"
                            ),
                        )
                        .with_location("field=last-column-id"),
                    );
                }
            }

            let mut highest_partition_id = None;
            let mut partition_id_signatures = BTreeMap::new();
            let mut partition_signature_ids = BTreeMap::new();
            for (spec_index, spec) in objects(metadata, "partition-specs").enumerate() {
                let partition_fields = spec
                    .get("fields")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_object)
                    .collect::<Vec<_>>();
                let partition_ids = partition_fields
                    .iter()
                    .filter_map(|field| integer(field, "field-id"))
                    .collect::<Vec<_>>();
                for duplicate in duplicates(&partition_ids) {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL106",
                            Severity::Error,
                            file,
                            format!("partition spec contains duplicate field ID {duplicate}"),
                        )
                        .with_location(format!("field=partition-specs[{spec_index}]")),
                    );
                }
                for field in partition_fields {
                    let location = format!("field=partition-specs[{spec_index}].fields");
                    match integer(field, "field-id") {
                        Some(partition_id) if partition_id > 0 => {
                            highest_partition_id =
                                Some(highest_partition_id.map_or(partition_id, |highest: i64| {
                                    highest.max(partition_id)
                                }));
                            if version >= 2 {
                                if let Some(signature) = partition_field_signature(field, version) {
                                    if partition_id_signatures
                                        .insert(partition_id, signature.clone())
                                        .is_some_and(|existing| existing != signature)
                                    {
                                        diagnostics.push(
                                            iceberg_diagnostic(
                                                "AL106",
                                                Severity::Error,
                                                file,
                                                format!(
                                                    "partition field ID {partition_id} is reused for a different field"
                                                ),
                                            )
                                            .with_location(location.clone()),
                                        );
                                    }
                                    if partition_signature_ids
                                        .insert(signature, partition_id)
                                        .is_some_and(|existing| existing != partition_id)
                                    {
                                        diagnostics.push(
                                            iceberg_diagnostic(
                                                "AL106",
                                                Severity::Error,
                                                file,
                                                "equivalent partition field was assigned a new field ID",
                                            )
                                            .with_location(location.clone()),
                                        );
                                    }
                                }
                            }
                        }
                        _ if version >= 2 => diagnostics.push(
                            iceberg_diagnostic(
                                "AL106",
                                Severity::Error,
                                file,
                                "v2+ partition field has no positive integer `field-id`",
                            )
                            .with_location(location.clone()),
                        ),
                        _ => {}
                    }
                    validate_source_ids(
                        &mut diagnostics,
                        file,
                        field,
                        &all_field_ids,
                        &location,
                        version,
                        "partition",
                    );
                }
            }
            if !has_non_empty_array(metadata, "partition-specs") {
                for field in metadata
                    .get("partition-spec")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_object)
                {
                    validate_source_ids(
                        &mut diagnostics,
                        file,
                        field,
                        &all_field_ids,
                        "field=partition-spec",
                        version,
                        "partition",
                    );
                }
            }
            if let (Some(last_partition_id), Some(highest_partition_id)) =
                (integer(metadata, "last-partition-id"), highest_partition_id)
            {
                if last_partition_id < highest_partition_id {
                    diagnostics.push(
                        iceberg_diagnostic(
                            "AL106",
                            Severity::Error,
                            file,
                            format!(
                                "`last-partition-id` {last_partition_id} is below assigned partition field ID {highest_partition_id}"
                            ),
                        )
                        .with_location("field=last-partition-id"),
                    );
                }
            }

            for (order_index, order) in objects(metadata, "sort-orders").enumerate() {
                for field in order
                    .get("fields")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_object)
                {
                    validate_source_ids(
                        &mut diagnostics,
                        file,
                        field,
                        &all_field_ids,
                        &format!("field=sort-orders[{order_index}].fields"),
                        version,
                        "sort",
                    );
                }
            }
        }
        diagnostics
    }
}

struct IcebergMetadataMaintenance;

impl Rule for IcebergMetadataMaintenance {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL107",
            name: "iceberg-metadata-maintenance",
            category: "maintenance",
            default_severity: Severity::Info,
            summary: "Large Iceberg snapshot and metadata histories should be reviewed.",
        }
    }

    fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (file, metadata) in iceberg_objects(dataset) {
            let snapshot_count = array_len(metadata, "snapshots");
            if config.rules.iceberg_max_snapshots > 0
                && snapshot_count > config.rules.iceberg_max_snapshots
            {
                diagnostics.push(
                    iceberg_diagnostic(
                        "AL107",
                        Severity::Info,
                        file,
                        format!(
                            "table metadata retains {snapshot_count} snapshots, above configured maximum {}",
                            config.rules.iceberg_max_snapshots
                        ),
                    )
                    .with_location("field=snapshots")
                    .with_help("review time-travel requirements and run snapshot expiration regularly"),
                );
            }

            let metadata_log_count = array_len(metadata, "metadata-log");
            if config.rules.iceberg_max_metadata_log_entries > 0
                && metadata_log_count > config.rules.iceberg_max_metadata_log_entries
            {
                diagnostics.push(
                    iceberg_diagnostic(
                        "AL107",
                        Severity::Info,
                        file,
                        format!(
                            "table metadata tracks {metadata_log_count} previous metadata files, above configured maximum {}",
                            config.rules.iceberg_max_metadata_log_entries
                        ),
                    )
                    .with_location("field=metadata-log")
                    .with_help(
                        "review write.metadata.previous-versions-max and write.metadata.delete-after-commit.enabled",
                    ),
                );
            }
        }
        diagnostics
    }
}

fn iceberg_objects(dataset: &Dataset) -> impl Iterator<Item = (&DatasetFile, &Map<String, Value>)> {
    dataset
        .files
        .iter()
        .filter(|file| file.format == Format::IcebergMetadata)
        .filter_map(|file| {
            file.iceberg_metadata
                .as_ref()
                .and_then(Value::as_object)
                .map(|metadata| (file, metadata))
        })
}

fn iceberg_diagnostic(
    rule_id: &'static str,
    severity: Severity,
    file: &DatasetFile,
    message: impl Into<String>,
) -> Diagnostic {
    let help = match rule_id {
        "AL102" => {
            "regenerate the metadata with a spec-compliant Iceberg writer rather than editing committed metadata"
        }
        "AL103" => {
            "repair the table through its catalog or Iceberg API so current and default IDs reference retained entries"
        }
        "AL104" => {
            "rewrite the invalid metadata through a compatible Iceberg implementation with unique identifiers"
        }
        "AL105" => {
            "repair snapshot metadata through the table catalog before using the affected snapshot"
        }
        "AL106" => {
            "repair schema evolution metadata without reusing field IDs or lowering ID high-water marks"
        }
        _ => "review the Iceberg table metadata and regenerate it with a compatible writer",
    };
    let category = if rule_id == "AL107" {
        "maintenance"
    } else {
        "iceberg"
    };
    Diagnostic::new(rule_id, severity, category, message)
        .with_path(file.path.clone())
        .with_help(help)
}

fn integer(metadata: &Map<String, Value>, key: &str) -> Option<i64> {
    metadata.get(key).and_then(Value::as_i64)
}

fn string<'a>(metadata: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(Value::as_str)
}

fn has_object(metadata: &Map<String, Value>, key: &str) -> bool {
    metadata.get(key).is_some_and(Value::is_object)
}

fn has_array(metadata: &Map<String, Value>, key: &str) -> bool {
    metadata.get(key).is_some_and(Value::is_array)
}

fn has_non_empty_array(metadata: &Map<String, Value>, key: &str) -> bool {
    metadata
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty())
}

fn array_len(metadata: &Map<String, Value>, key: &str) -> usize {
    metadata
        .get(key)
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn objects<'a>(
    metadata: &'a Map<String, Value>,
    key: &str,
) -> impl Iterator<Item = &'a Map<String, Value>> {
    metadata
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

fn schemas(metadata: &Map<String, Value>) -> Vec<&Map<String, Value>> {
    let mut schemas = objects(metadata, "schemas").collect::<Vec<_>>();
    if schemas.is_empty() {
        if let Some(schema) = metadata.get("schema").and_then(Value::as_object) {
            schemas.push(schema);
        }
    }
    schemas
}

fn object_ids(metadata: &Map<String, Value>, array_key: &str, id_key: &str) -> BTreeSet<i64> {
    objects(metadata, array_key)
        .filter_map(|object| integer(object, id_key))
        .collect()
}

fn duplicate_object_ids(
    metadata: &Map<String, Value>,
    array_key: &str,
    id_key: &str,
) -> BTreeSet<i64> {
    duplicates(
        &objects(metadata, array_key)
            .filter_map(|object| integer(object, id_key))
            .collect::<Vec<_>>(),
    )
}

fn duplicates(values: &[i64]) -> BTreeSet<i64> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .copied()
        .filter(|value| !seen.insert(*value))
        .collect()
}

fn require_non_empty_string(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    key: &str,
    description: &str,
) {
    if string(metadata, key).is_none_or(str::is_empty) {
        diagnostics.push(
            iceberg_diagnostic(
                "AL102",
                Severity::Error,
                file,
                format!("required {description} `{key}` is missing or empty"),
            )
            .with_location(format!("field={key}")),
        );
    }
}

fn require_integer(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    key: &str,
) {
    if integer(metadata, key).is_none() {
        diagnostics.push(missing_required(file, key));
    }
}

fn require_non_negative_integer(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    key: &str,
) {
    if integer(metadata, key).is_none_or(|value| value < 0) {
        diagnostics.push(
            iceberg_diagnostic(
                "AL102",
                Severity::Error,
                file,
                format!("required `{key}` must be a non-negative integer"),
            )
            .with_location(format!("field={key}")),
        );
    }
}

fn require_non_empty_array(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    key: &str,
) {
    if !has_non_empty_array(metadata, key) {
        diagnostics.push(
            iceberg_diagnostic(
                "AL102",
                Severity::Error,
                file,
                format!("required `{key}` must be a non-empty array"),
            )
            .with_location(format!("field={key}")),
        );
    }
}

fn require_array(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    key: &str,
) {
    if !has_array(metadata, key) {
        diagnostics.push(
            iceberg_diagnostic(
                "AL102",
                Severity::Error,
                file,
                format!("required `{key}` must be an array"),
            )
            .with_location(format!("field={key}")),
        );
    }
}

fn missing_required(file: &DatasetFile, key: &str) -> Diagnostic {
    iceberg_diagnostic(
        "AL102",
        Severity::Error,
        file,
        format!("required Iceberg metadata field `{key}` is missing or invalid"),
    )
    .with_location(format!("field={key}"))
}

fn require_uuid(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
) {
    if string(metadata, "table-uuid").is_none_or(|value| !is_uuid(value)) {
        diagnostics.push(
            iceberg_diagnostic(
                "AL102",
                Severity::Error,
                file,
                "required `table-uuid` is missing or is not a UUID",
            )
            .with_location("field=table-uuid"),
        );
    }
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| match index {
                8 | 13 | 18 | 23 => character == '-',
                _ => character.is_ascii_hexdigit(),
            })
}

fn is_absolute_location(value: &str) -> bool {
    value.starts_with('/')
        || value
            .split_once(':')
            .is_some_and(|(scheme, rest)| !scheme.is_empty() && !rest.is_empty())
}

fn check_reference(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    reference_key: &str,
    array_key: &str,
    id_key: &str,
) {
    let Some(reference) = integer(metadata, reference_key) else {
        return;
    };
    if !object_ids(metadata, array_key, id_key).contains(&reference) {
        diagnostics.push(invalid_reference(file, reference_key, reference, array_key));
    }
}

fn invalid_reference(
    file: &DatasetFile,
    reference_key: &str,
    reference: i64,
    target: &str,
) -> Diagnostic {
    iceberg_diagnostic(
        "AL103",
        Severity::Error,
        file,
        format!("`{reference_key}` value {reference} does not reference an entry in `{target}`"),
    )
    .with_location(format!("field={reference_key}"))
}

fn current_snapshot_id(metadata: &Map<String, Value>) -> Option<i64> {
    integer(metadata, "current-snapshot-id").filter(|snapshot_id| *snapshot_id != -1)
}

fn check_snapshot_log(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    metadata: &Map<String, Value>,
    snapshot_ids: &BTreeSet<i64>,
    last_updated_ms: Option<i64>,
) {
    let entries = objects(metadata, "snapshot-log").collect::<Vec<_>>();
    let mut previous_timestamp = None;
    for (index, entry) in entries.iter().enumerate() {
        let location = format!("field=snapshot-log[{index}]");
        let Some(timestamp_ms) = integer(entry, "timestamp-ms") else {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    "snapshot log entry has no integer `timestamp-ms`",
                )
                .with_location(location.clone()),
            );
            continue;
        };
        if previous_timestamp.is_some_and(|previous| timestamp_ms <= previous) {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    "snapshot log timestamps are not strictly increasing",
                )
                .with_location(location.clone()),
            );
        }
        if last_updated_ms.is_some_and(|last_updated_ms| timestamp_ms > last_updated_ms) {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    "snapshot log timestamp is later than `last-updated-ms`",
                )
                .with_location(location.clone()),
            );
        }
        previous_timestamp = Some(timestamp_ms);

        let Some(snapshot_id) = integer(entry, "snapshot-id") else {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    "snapshot log entry has no integer `snapshot-id`",
                )
                .with_location(location),
            );
            continue;
        };
        if !snapshot_ids.contains(&snapshot_id) {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    format!("snapshot log references expired or unknown snapshot ID {snapshot_id}"),
                )
                .with_location(location),
            );
        }
    }

    if let (Some(current_snapshot_id), Some(last_entry)) =
        (current_snapshot_id(metadata), entries.last())
    {
        if integer(last_entry, "snapshot-id") != Some(current_snapshot_id) {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL105",
                    Severity::Error,
                    file,
                    "last snapshot log entry does not match the current snapshot",
                )
                .with_location("field=snapshot-log"),
            );
        }
    }
}

fn schema_field_id_values(schema: &Map<String, Value>) -> Vec<Option<i64>> {
    let mut ids = Vec::new();
    collect_fields(schema.get("fields"), &mut ids);
    ids
}

fn collect_fields(fields: Option<&Value>, ids: &mut Vec<Option<i64>>) {
    for field in fields
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        ids.push(integer(field, "id"));
        collect_type(field.get("type"), ids);
    }
}

fn collect_type(field_type: Option<&Value>, ids: &mut Vec<Option<i64>>) {
    let Some(field_type) = field_type.and_then(Value::as_object) else {
        return;
    };
    match string(field_type, "type") {
        Some("struct") => collect_fields(field_type.get("fields"), ids),
        Some("list") => {
            ids.push(integer(field_type, "element-id"));
            collect_type(field_type.get("element"), ids);
        }
        Some("map") => {
            ids.push(integer(field_type, "key-id"));
            ids.push(integer(field_type, "value-id"));
            collect_type(field_type.get("key"), ids);
            collect_type(field_type.get("value"), ids);
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_source_ids(
    diagnostics: &mut Vec<Diagnostic>,
    file: &DatasetFile,
    field: &Map<String, Value>,
    known_field_ids: &BTreeSet<i64>,
    location: &str,
    version: i64,
    field_kind: &str,
) {
    let Some(source_ids) = parse_source_ids(field, version) else {
        diagnostics.push(
            iceberg_diagnostic(
                "AL106",
                Severity::Error,
                file,
                format!(
                    "{field_kind} field must contain exactly one valid `source-id` or v3 `source-ids`"
                ),
            )
            .with_location(location),
        );
        return;
    };
    for source_id in source_ids {
        if !known_field_ids.contains(&source_id) {
            diagnostics.push(
                iceberg_diagnostic(
                    "AL106",
                    Severity::Error,
                    file,
                    format!("{field_kind} field references unknown source ID {source_id}"),
                )
                .with_location(location),
            );
        }
    }
}

fn parse_source_ids(field: &Map<String, Value>, version: i64) -> Option<Vec<i64>> {
    match (field.get("source-id"), field.get("source-ids")) {
        (Some(source_id), None) => source_id.as_i64().map(|source_id| vec![source_id]),
        (None, Some(source_ids)) if version >= 3 => source_ids.as_array().and_then(|source_ids| {
            (!source_ids.is_empty())
                .then(|| {
                    source_ids
                        .iter()
                        .map(Value::as_i64)
                        .collect::<Option<Vec<_>>>()
                })
                .flatten()
        }),
        _ => None,
    }
}

fn partition_field_signature(field: &Map<String, Value>, version: i64) -> Option<String> {
    Some(format!(
        "{:?}|{}|{}",
        parse_source_ids(field, version)?,
        string(field, "transform")?,
        string(field, "name")?
    ))
}
