use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::{
    dataset::{Dataset, DatasetFile, Format, GeoParquetMetadata},
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    rules::RuleMetadata,
    LintConfig,
};

const SUPPORTED_ENCODINGS: &[&str] = &[
    "WKB",
    "point",
    "linestring",
    "polygon",
    "multipoint",
    "multilinestring",
    "multipolygon",
];
const GEOMETRY_TYPES: &[&str] = &[
    "GeometryCollection",
    "Point",
    "LineString",
    "Polygon",
    "MultiPoint",
    "MultiLineString",
    "MultiPolygon",
];

pub(crate) fn parse_metadata(values: &[Option<&str>]) -> Option<GeoParquetMetadata> {
    let [raw] = values else {
        return (!values.is_empty()).then(|| GeoParquetMetadata {
            document: None,
            error: Some(format!(
                "Parquet metadata contains {} `geo` entries; expected exactly one",
                values.len()
            )),
        });
    };
    let Some(raw) = raw else {
        return Some(GeoParquetMetadata {
            document: None,
            error: Some("Parquet `geo` metadata key has no JSON value".to_string()),
        });
    };
    Some(match serde_json::from_str(raw) {
        Ok(document) => GeoParquetMetadata {
            document: Some(document),
            error: None,
        },
        Err(error) => GeoParquetMetadata {
            document: None,
            error: Some(format!("`geo` metadata is not valid JSON: {error}")),
        },
    })
}

pub(crate) fn register_rules(registry: &mut RuleRegistry) {
    registry.register(InvalidGeoParquetMetadata);
    registry.register(InvalidGeoParquetColumns);
    registry.register(InvalidGeoParquetSpatialMetadata);
    registry.register(MissingGeoParquetPruningMetadata);
}

struct InvalidGeoParquetMetadata;

impl Rule for InvalidGeoParquetMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL018",
            name: "invalid-geoparquet-metadata",
            category: "geoparquet",
            default_severity: Severity::Error,
            summary: "GeoParquet metadata must be valid, supported, and complete.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        geoparquet_files(dataset)
            .flat_map(|(file, metadata)| {
                let mut diagnostics = Vec::new();
                if let Some(error) = &metadata.error {
                    diagnostics.push(geo_diagnostic(
                        "AL018",
                        Severity::Error,
                        file,
                        error,
                    ));
                    return diagnostics;
                }
                let Some(document) = metadata.document.as_ref().and_then(Value::as_object) else {
                    diagnostics.push(geo_diagnostic(
                        "AL018",
                        Severity::Error,
                        file,
                        "`geo` metadata root must be a JSON object",
                    ));
                    return diagnostics;
                };

                match document.get("version").and_then(Value::as_str) {
                    None => diagnostics.push(geo_diagnostic(
                        "AL018",
                        Severity::Error,
                        file,
                        "`geo.version` must be a semantic-version string",
                    )),
                    Some(version) if !is_supported_version(version) => diagnostics.push(
                        geo_diagnostic(
                            "AL018",
                            Severity::Error,
                            file,
                            format!(
                                "GeoParquet version `{version}` is malformed or has an unsupported major version"
                            ),
                        ),
                    ),
                    Some(_) => {}
                }
                if !document
                    .get("primary_column")
                    .and_then(Value::as_str)
                    .is_some_and(|column| !column.is_empty())
                {
                    diagnostics.push(geo_diagnostic(
                        "AL018",
                        Severity::Error,
                        file,
                        "`geo.primary_column` must be a non-empty string",
                    ));
                }
                if !document
                    .get("columns")
                    .and_then(Value::as_object)
                    .is_some_and(|columns| !columns.is_empty())
                {
                    diagnostics.push(geo_diagnostic(
                        "AL018",
                        Severity::Error,
                        file,
                        "`geo.columns` must be a non-empty object",
                    ));
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidGeoParquetColumns;

impl Rule for InvalidGeoParquetColumns {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL019",
            name: "invalid-geoparquet-columns",
            category: "geoparquet",
            default_severity: Severity::Error,
            summary:
                "GeoParquet geometry columns must match the Parquet schema and encoding rules.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        geoparquet_documents(dataset)
            .flat_map(|(file, document)| {
                let mut diagnostics = Vec::new();
                let Some(columns) = document.get("columns").and_then(Value::as_object) else {
                    return diagnostics;
                };
                if let Some(primary) = document.get("primary_column").and_then(Value::as_str) {
                    if !columns.contains_key(primary) {
                        diagnostics.push(geo_diagnostic(
                            "AL019",
                            Severity::Error,
                            file,
                            format!(
                                "primary geometry column `{primary}` is absent from `geo.columns`"
                            ),
                        ));
                    }
                }
                let version = document.get("version").and_then(Value::as_str);
                for (column_name, column) in columns {
                    let Some(column) = column.as_object() else {
                        diagnostics.push(
                            geo_diagnostic(
                                "AL019",
                                Severity::Error,
                                file,
                                format!("geometry column `{column_name}` metadata must be an object"),
                            )
                            .with_location(format!("column={column_name}")),
                        );
                        continue;
                    };
                    if column_name.is_empty() {
                        diagnostics.push(geo_diagnostic(
                            "AL019",
                            Severity::Error,
                            file,
                            "GeoParquet geometry column names must not be empty",
                        ));
                    }
                    let schema_field = file.schema.as_ref().and_then(|schema| {
                        schema
                            .fields
                            .iter()
                            .find(|field| field.name == *column_name)
                    });
                    if schema_field.is_none() {
                        diagnostics.push(
                            geo_diagnostic(
                                "AL019",
                                Severity::Error,
                                file,
                                format!(
                                    "geometry column `{column_name}` is absent from the root Parquet schema"
                                ),
                            )
                            .with_location(format!("column={column_name}")),
                        );
                    }

                    let encoding = column.get("encoding").and_then(Value::as_str);
                    let encoding_is_valid = encoding.is_some_and(|encoding| {
                        SUPPORTED_ENCODINGS.contains(&encoding)
                            && (encoding == "WKB" || supports_native_encoding(version))
                    });
                    if !encoding_is_valid {
                        diagnostics.push(
                            geo_diagnostic(
                                "AL019",
                                Severity::Error,
                                file,
                                format!(
                                    "geometry column `{column_name}` has a missing or unsupported encoding"
                                ),
                            )
                            .with_location(format!("column={column_name}")),
                        );
                    }

                    match column.get("geometry_types").and_then(Value::as_array) {
                        None => diagnostics.push(
                            geo_diagnostic(
                                "AL019",
                                Severity::Error,
                                file,
                                format!(
                                    "geometry column `{column_name}` must declare `geometry_types`"
                                ),
                            )
                            .with_location(format!("column={column_name}")),
                        ),
                        Some(geometry_types) => {
                            let mut seen = BTreeSet::new();
                            for geometry_type in geometry_types {
                                let valid = geometry_type.as_str().is_some_and(|geometry_type| {
                                    valid_geometry_type(geometry_type)
                                        && seen.insert(geometry_type.to_string())
                                });
                                if !valid {
                                    diagnostics.push(
                                        geo_diagnostic(
                                            "AL019",
                                            Severity::Error,
                                            file,
                                            format!(
                                                "geometry column `{column_name}` has an invalid or duplicate geometry type"
                                            ),
                                        )
                                        .with_location(format!("column={column_name}")),
                                    );
                                }
                            }
                        }
                    }

                    if encoding == Some("WKB") && !wkb_column_is_binary(file, column_name) {
                        diagnostics.push(
                            geo_diagnostic(
                                "AL019",
                                Severity::Error,
                                file,
                                format!(
                                    "WKB geometry column `{column_name}` is not stored as Parquet BYTE_ARRAY"
                                ),
                            )
                            .with_location(format!("column={column_name}")),
                        );
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidGeoParquetSpatialMetadata;

impl Rule for InvalidGeoParquetSpatialMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL020",
            name: "invalid-geoparquet-spatial-metadata",
            category: "geoparquet",
            default_severity: Severity::Error,
            summary: "GeoParquet CRS, bounds, edges, orientation, and coverings must be valid.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        geoparquet_columns(dataset)
            .flat_map(|(file, column_name, column)| {
                let mut diagnostics = Vec::new();
                if column
                    .get("crs")
                    .is_some_and(|crs| !(crs.is_object() || crs.is_null()))
                {
                    diagnostics.push(spatial_diagnostic(
                        file,
                        column_name,
                        "`crs` must be a PROJJSON object or null",
                    ));
                }
                if column
                    .get("epoch")
                    .is_some_and(|epoch| !epoch.is_number())
                {
                    diagnostics.push(spatial_diagnostic(
                        file,
                        column_name,
                        "`epoch` must be a number",
                    ));
                }
                if column
                    .get("orientation")
                    .is_some_and(|orientation| orientation.as_str() != Some("counterclockwise"))
                {
                    diagnostics.push(spatial_diagnostic(
                        file,
                        column_name,
                        "`orientation` must be `counterclockwise` when present",
                    ));
                }
                if column.get("edges").is_some_and(|edges| {
                    !matches!(edges.as_str(), Some("planar" | "spherical"))
                }) {
                    diagnostics.push(spatial_diagnostic(
                        file,
                        column_name,
                        "`edges` must be `planar` or `spherical`",
                    ));
                }
                if column.get("bbox").is_some_and(|bbox| !valid_bbox(bbox)) {
                    diagnostics.push(spatial_diagnostic(
                        file,
                        column_name,
                        "`bbox` must contain ordered numeric minima and maxima for two or three dimensions",
                    ));
                }
                if let Some(covering) = column.get("covering") {
                    if let Some(error) = covering_error(file, covering) {
                        diagnostics.push(spatial_diagnostic(file, column_name, error));
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct MissingGeoParquetPruningMetadata;

impl Rule for MissingGeoParquetPruningMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL021",
            name: "missing-geoparquet-pruning-metadata",
            category: "performance",
            default_severity: Severity::Info,
            summary: "GeoParquet bounds can improve file and row-group spatial pruning.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        geoparquet_columns(dataset)
            .filter(|(_, _, column)| !column.contains_key("bbox") && !column.contains_key("covering"))
            .map(|(file, column_name, _)| {
                geo_diagnostic(
                    "AL021",
                    Severity::Info,
                    file,
                    format!(
                        "geometry column `{column_name}` has neither file bounds nor a bounding-box covering"
                    ),
                )
                .with_location(format!("column={column_name}"))
            })
            .collect()
    }
}

fn geoparquet_files(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &GeoParquetMetadata)> {
    dataset
        .files
        .iter()
        .filter(|file| file.format == Format::Parquet)
        .filter_map(|file| {
            file.geoparquet_metadata
                .as_ref()
                .map(|metadata| (file, metadata))
        })
}

fn geoparquet_documents(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &Map<String, Value>)> {
    geoparquet_files(dataset).filter_map(|(file, metadata)| {
        metadata
            .document
            .as_ref()
            .and_then(Value::as_object)
            .map(|document| (file, document))
    })
}

fn geoparquet_columns(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &str, &Map<String, Value>)> {
    geoparquet_documents(dataset).flat_map(|(file, document)| {
        document
            .get("columns")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(move |columns| {
                columns.iter().filter_map(move |(name, column)| {
                    column
                        .as_object()
                        .map(|column| (file, name.as_str(), column))
                })
            })
    })
}

fn is_supported_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(major) = parts.next().filter(|part| valid_version_component(part)) else {
        return false;
    };
    let Some(_minor) = parts.next().filter(|part| valid_version_component(part)) else {
        return false;
    };
    let Some(_patch) = parts.next().filter(|part| valid_version_component(part)) else {
        return false;
    };
    parts.next().is_none() && major == "1"
}

fn valid_version_component(component: &str) -> bool {
    !component.is_empty()
        && component.bytes().all(|byte| byte.is_ascii_digit())
        && (component.len() == 1 || !component.starts_with('0'))
}

fn supports_native_encoding(version: Option<&str>) -> bool {
    version
        .and_then(|version| version.split('.').nth(1))
        .and_then(|minor| minor.parse::<u64>().ok())
        .is_some_and(|minor| minor >= 1)
}

fn valid_geometry_type(geometry_type: &str) -> bool {
    let base = geometry_type.strip_suffix(" Z").unwrap_or(geometry_type);
    GEOMETRY_TYPES.contains(&base)
}

fn wkb_column_is_binary(file: &DatasetFile, column_name: &str) -> bool {
    let physical_types = file
        .row_groups
        .iter()
        .flat_map(|group| &group.columns)
        .filter(|column| column.path == column_name)
        .map(|column| column.physical_type.as_str())
        .collect::<Vec<_>>();
    if !physical_types.is_empty() {
        return physical_types
            .iter()
            .all(|physical_type| *physical_type == "BYTE_ARRAY");
    }
    file.schema
        .as_ref()
        .and_then(|schema| schema.fields.iter().find(|field| field.name == column_name))
        .is_some_and(|field| matches!(field.data_type.as_str(), "Binary" | "LargeBinary"))
}

fn valid_bbox(value: &Value) -> bool {
    let Some(values) = value.as_array() else {
        return false;
    };
    let dimensions = match values.len() {
        4 => 2,
        6 => 3,
        _ => return false,
    };
    let Some(values) = values.iter().map(Value::as_f64).collect::<Option<Vec<_>>>() else {
        return false;
    };
    (0..dimensions).all(|dimension| values[dimension] <= values[dimension + dimensions])
}

fn covering_error(file: &DatasetFile, covering: &Value) -> Option<String> {
    let Some(covering) = covering.as_object() else {
        return Some("`covering` must be an object".to_string());
    };
    let Some(bbox) = covering.get("bbox").and_then(Value::as_object) else {
        return Some("`covering` must contain a `bbox` object".to_string());
    };
    let mut root = None;
    for coordinate in ["xmin", "ymin", "xmax", "ymax"] {
        let Some(path) = bbox.get(coordinate).and_then(Value::as_array) else {
            return Some(format!(
                "`covering.bbox` must define a two-part path for `{coordinate}`"
            ));
        };
        let root_is_missing = match path.first().and_then(Value::as_str) {
            Some(root) => root.is_empty(),
            None => true,
        };
        if path.len() != 2 || root_is_missing || path[1].as_str() != Some(coordinate) {
            return Some(format!(
                "`covering.bbox.{coordinate}` must be `[column, \"{coordinate}\"]`"
            ));
        }
        let current_root = path[0].as_str().expect("validated string");
        if root.is_some_and(|root| root != current_root) {
            return Some("all bounding-box covering paths must use one root column".to_string());
        }
        root = Some(current_root);
    }
    let root = root.expect("required covering paths set a root");
    if !file
        .schema
        .as_ref()
        .is_some_and(|schema| schema.fields.iter().any(|field| field.name == root))
    {
        return Some(format!(
            "bounding-box covering root column `{root}` is absent from the Parquet schema"
        ));
    }
    None
}

fn spatial_diagnostic(
    file: &DatasetFile,
    column_name: &str,
    message: impl Into<String>,
) -> Diagnostic {
    geo_diagnostic("AL020", Severity::Error, file, message)
        .with_location(format!("column={column_name}"))
}

fn geo_diagnostic(
    rule_id: &'static str,
    severity: Severity,
    file: &DatasetFile,
    message: impl Into<String>,
) -> Diagnostic {
    let help = match rule_id {
        "AL018" => "write `geo` metadata conforming to a GeoParquet 1.x metadata schema",
        "AL019" => {
            "align geometry column names, encodings, geometry types, and Parquet physical types"
        }
        "AL020" => "rewrite invalid spatial metadata using the GeoParquet 1.1 schema",
        _ => {
            "write file-level bounds or a valid bounding-box covering when spatial pruning matters"
        }
    };
    Diagnostic::new(
        rule_id,
        severity,
        if rule_id == "AL021" {
            "performance"
        } else {
            "geoparquet"
        },
        message,
    )
    .with_path(file.path.clone())
    .with_help(help)
}
