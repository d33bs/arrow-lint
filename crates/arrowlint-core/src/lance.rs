use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};
use prost::Message;

use crate::{
    dataset::{
        Dataset, DatasetFile, Format, LanceBasePath, LanceDataFile, LanceDataStorageFormat,
        LanceDeletionFile, LanceFragment, LanceManifest, LanceMetadata,
    },
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    rules::RuleMetadata,
    LintConfig,
};

const MANIFEST_MAGIC: &[u8; 4] = b"LANC";
const FLAG_DELETION_FILES: u64 = 1;
const FLAG_USE_V2_FORMAT_DEPRECATED: u64 = 4;
const FLAG_TABLE_CONFIG: u64 = 8;
const FLAG_BASE_PATHS: u64 = 16;
const FLAG_UNSTABLE_DATA_OVERLAY_FILES: u64 = 64;
const FLAG_UNKNOWN: u64 = 128;

pub(crate) fn scan_dataset(path: &Path) -> Result<DatasetFile> {
    let metadata = inspect_dataset(path)?;
    let size_bytes = metadata
        .selected_manifest_path
        .as_deref()
        .and_then(|manifest_path| fs::metadata(manifest_path).ok())
        .map_or(0, |file_metadata| file_metadata.len());
    let num_rows = metadata.manifest.as_ref().and_then(|manifest| {
        manifest
            .fragments
            .iter()
            .try_fold(0_i64, |total, fragment| {
                let deleted = fragment
                    .deletion_file
                    .as_ref()
                    .map_or(0, |deletion| deletion.num_deleted_rows);
                let live_rows = fragment.physical_rows.saturating_sub(deleted);
                i64::try_from(live_rows)
                    .ok()
                    .and_then(|live_rows| total.checked_add(live_rows))
            })
    });
    let mut summary = BTreeMap::new();
    if let Some(manifest) = &metadata.manifest {
        summary.insert("lance.version".to_string(), manifest.version.to_string());
        summary.insert(
            "lance.fragments".to_string(),
            manifest.fragments.len().to_string(),
        );
        if let Some(data_format) = &manifest.data_format {
            summary.insert(
                "lance.data_format".to_string(),
                format!("{} {}", data_format.file_format, data_format.version),
            );
        }
    }

    Ok(DatasetFile {
        path: path.display().to_string(),
        format: Format::LanceDataset,
        size_bytes,
        num_rows,
        schema: None,
        metadata: summary,
        row_groups: Vec::new(),
        iceberg_metadata: None,
        lance_metadata: Some(metadata),
    })
}

fn inspect_dataset(root: &Path) -> Result<LanceMetadata> {
    let versions_directory = root.join("_versions");
    let mut metadata = LanceMetadata {
        versions_directory_present: versions_directory.is_dir(),
        ..LanceMetadata::default()
    };
    if !metadata.versions_directory_present {
        metadata.errors.push(format!(
            "required versions directory is missing: {}",
            versions_directory.display()
        ));
        return Ok(metadata);
    }

    let mut manifests = Vec::new();
    for entry in fs::read_dir(&versions_directory)
        .with_context(|| format!("failed to read {}", versions_directory.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        if filename.starts_with('d') && filename.ends_with(".manifest") {
            continue;
        }
        if !filename.ends_with(".manifest") {
            continue;
        }
        match parse_manifest_filename(filename) {
            Some((scheme, version)) => manifests.push((version, scheme, entry.path())),
            None => metadata
                .errors
                .push(format!("invalid manifest filename `{filename}`")),
        }
    }

    metadata.manifest_count = manifests.len();
    let naming_schemes = manifests
        .iter()
        .map(|(_, scheme, _)| scheme.as_str().to_string())
        .collect::<BTreeSet<_>>();
    metadata.naming_schemes = naming_schemes.iter().cloned().collect();
    if naming_schemes.len() > 1 {
        metadata
            .errors
            .push("mixed V1 and V2 manifest naming schemes were found in `_versions`".to_string());
    }

    let Some((filename_version, _, manifest_path)) =
        manifests.into_iter().max_by_key(|(version, _, _)| *version)
    else {
        metadata
            .errors
            .push("no attached Lance manifest was found in `_versions`".to_string());
        return Ok(metadata);
    };
    metadata.filename_version = Some(filename_version);
    metadata.selected_manifest_path = Some(manifest_path.display().to_string());

    match read_manifest(&manifest_path) {
        Ok(manifest) => metadata.manifest = Some(manifest),
        Err(error) => metadata.errors.push(format!(
            "failed to decode latest manifest `{}`: {error:#}",
            manifest_path.display()
        )),
    }
    Ok(metadata)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestNamingScheme {
    V1,
    V2,
}

impl ManifestNamingScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

fn parse_manifest_filename(filename: &str) -> Option<(ManifestNamingScheme, u64)> {
    let stem = filename.strip_suffix(".manifest")?;
    let encoded_version = stem.parse::<u64>().ok()?;
    if stem.len() == 20 {
        Some((
            ManifestNamingScheme::V2,
            u64::MAX.checked_sub(encoded_version)?,
        ))
    } else {
        Some((ManifestNamingScheme::V1, encoded_version))
    }
}

fn read_manifest(path: &Path) -> Result<LanceManifest> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() < 20 {
        anyhow::bail!("file is smaller than the minimum manifest envelope");
    }
    if !bytes.ends_with(MANIFEST_MAGIC) {
        anyhow::bail!("footer magic is not `LANC`");
    }
    let footer_start = bytes.len() - 16;
    let manifest_position = i64::from_le_bytes(
        bytes[footer_start..footer_start + 8]
            .try_into()
            .expect("eight-byte manifest position"),
    );
    if manifest_position < 0 {
        anyhow::bail!("manifest position is negative");
    }
    let manifest_position = manifest_position as usize;
    if manifest_position + 4 > footer_start {
        anyhow::bail!("manifest position is outside the file");
    }
    let major_version = i16::from_le_bytes(
        bytes[footer_start + 8..footer_start + 10]
            .try_into()
            .expect("two-byte major version"),
    );
    let minor_version = i16::from_le_bytes(
        bytes[footer_start + 10..footer_start + 12]
            .try_into()
            .expect("two-byte minor version"),
    );
    if (major_version, minor_version) != (0, 1) {
        anyhow::bail!(
            "unsupported manifest envelope version {major_version}.{minor_version}; expected 0.1"
        );
    }
    let recorded_length = u32::from_le_bytes(
        bytes[manifest_position..manifest_position + 4]
            .try_into()
            .expect("four-byte message length"),
    ) as usize;
    let message_start = manifest_position + 4;
    let actual_length = footer_start.saturating_sub(message_start);
    if recorded_length != actual_length {
        anyhow::bail!(
            "manifest length mismatch: footer records {recorded_length} bytes, found {actual_length}"
        );
    }
    let manifest = PbManifest::decode(&bytes[message_start..footer_start])
        .context("invalid Lance manifest protobuf")?;
    Ok(manifest.into())
}

impl From<PbManifest> for LanceManifest {
    fn from(manifest: PbManifest) -> Self {
        Self {
            version: manifest.version,
            reader_feature_flags: manifest.reader_feature_flags,
            writer_feature_flags: manifest.writer_feature_flags,
            max_fragment_id: manifest.max_fragment_id,
            transaction_file: manifest.transaction_file,
            data_format: manifest.data_format.map(Into::into),
            config: manifest.config.into_iter().collect(),
            base_paths: manifest.base_paths.into_iter().map(Into::into).collect(),
            fragments: manifest.fragments.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<PbDataStorageFormat> for LanceDataStorageFormat {
    fn from(data_format: PbDataStorageFormat) -> Self {
        Self {
            file_format: data_format.file_format,
            version: data_format.version,
        }
    }
}

impl From<PbBasePath> for LanceBasePath {
    fn from(base_path: PbBasePath) -> Self {
        Self {
            id: base_path.id,
            is_dataset_root: base_path.is_dataset_root,
            path: base_path.path,
        }
    }
}

impl From<PbFragment> for LanceFragment {
    fn from(fragment: PbFragment) -> Self {
        Self {
            id: fragment.id,
            files: fragment.files.into_iter().map(Into::into).collect(),
            deletion_file: fragment.deletion_file.map(Into::into),
            physical_rows: fragment.physical_rows,
            overlay_count: fragment.overlays.len(),
        }
    }
}

impl From<PbDataFile> for LanceDataFile {
    fn from(file: PbDataFile) -> Self {
        Self {
            path: file.path,
            fields: file.fields,
            column_indices: file.column_indices,
            file_major_version: file.file_major_version,
            file_minor_version: file.file_minor_version,
            file_size_bytes: file.file_size_bytes,
            base_id: file.base_id,
        }
    }
}

impl From<PbDeletionFile> for LanceDeletionFile {
    fn from(file: PbDeletionFile) -> Self {
        Self {
            file_type: file.file_type,
            read_version: file.read_version,
            id: file.id,
            num_deleted_rows: file.num_deleted_rows,
            base_id: file.base_id,
        }
    }
}

pub(crate) fn register_rules(registry: &mut RuleRegistry) {
    registry.register(InvalidManifest);
    registry.register(InvalidFragmentIdentifiers);
    registry.register(InvalidDataFiles);
    registry.register(InvalidDeletionFiles);
    registry.register(IncompatibleFeatures);
    registry.register(MissingReferences);
    registry.register(MaintenanceOpportunities);
}

struct InvalidManifest;

impl Rule for InvalidManifest {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL201",
            name: "invalid-lance-manifest",
            category: "lance",
            default_severity: Severity::Error,
            summary: "Lance datasets must have a decodable, consistently named latest manifest.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_files(dataset)
            .flat_map(|(file, metadata)| {
                let mut diagnostics = metadata
                    .errors
                    .iter()
                    .map(|error| lance_diagnostic("AL201", Severity::Error, file, error))
                    .collect::<Vec<_>>();
                if let (Some(filename_version), Some(manifest)) =
                    (metadata.filename_version, metadata.manifest.as_ref())
                {
                    if manifest.version != filename_version {
                        diagnostics.push(lance_diagnostic(
                            "AL201",
                            Severity::Error,
                            file,
                            format!(
                                "manifest filename identifies version {filename_version}, but its payload records version {}",
                                manifest.version
                            ),
                        ));
                    }
                    if manifest.version == 0 {
                        diagnostics.push(lance_diagnostic(
                            "AL201",
                            Severity::Error,
                            file,
                            "attached manifest version must be greater than zero",
                        ));
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidFragmentIdentifiers;

impl Rule for InvalidFragmentIdentifiers {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL202",
            name: "invalid-lance-fragment-identifiers",
            category: "lance",
            default_severity: Severity::Error,
            summary: "Lance fragment identifiers must be unique and respect their high-water mark.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, _, manifest)| {
                let mut diagnostics = Vec::new();
                let mut fragment_ids = BTreeSet::new();
                for fragment in &manifest.fragments {
                    if !fragment_ids.insert(fragment.id) {
                        diagnostics.push(
                            lance_diagnostic(
                                "AL202",
                                Severity::Error,
                                file,
                                format!("fragment ID {} is duplicated", fragment.id),
                            )
                            .with_location(format!("fragment={}", fragment.id)),
                        );
                    }
                }
                if let (Some(max_fragment_id), Some(current_max)) = (
                    manifest.max_fragment_id,
                    manifest.fragments.iter().map(|fragment| fragment.id).max(),
                ) {
                    if current_max > u64::from(max_fragment_id) {
                        diagnostics.push(lance_diagnostic(
                            "AL202",
                            Severity::Error,
                            file,
                            format!(
                                "max_fragment_id {max_fragment_id} is below current fragment ID {current_max}"
                            ),
                        ));
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidDataFiles;

impl Rule for InvalidDataFiles {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL203",
            name: "invalid-lance-data-files",
            category: "lance",
            default_severity: Severity::Error,
            summary: "Lance data-file paths and field mappings must satisfy manifest invariants.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, _, manifest)| {
                let mut diagnostics = Vec::new();
                for fragment in &manifest.fragments {
                    let mut active_fields = BTreeSet::new();
                    for data_file in &fragment.files {
                        let location =
                            format!("fragment={},data_file={}", fragment.id, data_file.path);
                        if !is_safe_relative_path(&data_file.path) {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL203",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "fragment {} has invalid relative data-file path `{}`",
                                        fragment.id, data_file.path
                                    ),
                                )
                                .with_location(location.clone()),
                            );
                        }
                        if data_file.fields.contains(&-1) {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL203",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "data file `{}` persists forbidden field ID -1",
                                        data_file.path
                                    ),
                                )
                                .with_location(location.clone()),
                            );
                        }
                        for field_id in data_file.fields.iter().copied().filter(|id| *id >= 0) {
                            if !active_fields.insert(field_id) {
                                diagnostics.push(
                                    lance_diagnostic(
                                        "AL203",
                                        Severity::Error,
                                        file,
                                        format!(
                                            "active field ID {field_id} appears in multiple data files for fragment {}",
                                            fragment.id
                                        ),
                                    )
                                    .with_location(location.clone()),
                                );
                            }
                        }
                        if data_file.file_major_version > 0
                            && data_file.column_indices.len() != data_file.fields.len()
                        {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL203",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "v{}.{} data file `{}` has {} fields but {} column indices",
                                        data_file.file_major_version,
                                        data_file.file_minor_version,
                                        data_file.path,
                                        data_file.fields.len(),
                                        data_file.column_indices.len()
                                    ),
                                )
                                .with_location(location.clone()),
                            );
                        }
                        let mut column_indices = BTreeSet::new();
                        for column_index in data_file
                            .column_indices
                            .iter()
                            .copied()
                            .filter(|index| *index >= 0)
                        {
                            if !column_indices.insert(column_index) {
                                diagnostics.push(
                                    lance_diagnostic(
                                        "AL203",
                                        Severity::Error,
                                        file,
                                        format!(
                                            "data file `{}` repeats column index {column_index}",
                                            data_file.path
                                        ),
                                    )
                                    .with_location(location.clone()),
                                );
                            }
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidDeletionFiles;

impl Rule for InvalidDeletionFiles {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL204",
            name: "invalid-lance-deletion-files",
            category: "lance",
            default_severity: Severity::Error,
            summary: "Lance deletion metadata must use valid types, versions, and row counts.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, _, manifest)| {
                manifest
                    .fragments
                    .iter()
                    .filter_map(|fragment| {
                        fragment
                            .deletion_file
                            .as_ref()
                            .map(|deletion| (fragment, deletion))
                    })
                    .flat_map(|(fragment, deletion)| {
                        let mut diagnostics = Vec::new();
                        let location = format!("fragment={}", fragment.id);
                        if !matches!(deletion.file_type, 0 | 1) {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL204",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "fragment {} uses unknown deletion-file type {}",
                                        fragment.id, deletion.file_type
                                    ),
                                )
                                .with_location(location.clone()),
                            );
                        }
                        if deletion.read_version > manifest.version {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL204",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "fragment {} deletion file was read from future version {}",
                                        fragment.id, deletion.read_version
                                    ),
                                )
                                .with_location(location.clone()),
                            );
                        }
                        if deletion.num_deleted_rows > fragment.physical_rows {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL204",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "fragment {} deletes {} rows but contains only {} physical rows",
                                        fragment.id,
                                        deletion.num_deleted_rows,
                                        fragment.physical_rows
                                    ),
                                )
                                .with_location(location),
                            );
                        }
                        diagnostics
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct IncompatibleFeatures;

impl Rule for IncompatibleFeatures {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL205",
            name: "incompatible-lance-features",
            category: "interoperability",
            default_severity: Severity::Error,
            summary: "Lance feature flags must be known and consistent with manifest content.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, _, manifest)| {
                let mut diagnostics = Vec::new();
                for (kind, flags) in [
                    ("reader", manifest.reader_feature_flags),
                    ("writer", manifest.writer_feature_flags),
                ] {
                    let unknown = flags & !(FLAG_UNKNOWN - 1);
                    if unknown != 0 {
                        diagnostics.push(lance_diagnostic(
                            "AL205",
                            Severity::Error,
                            file,
                            format!("unknown {kind} feature flag bits are set: 0x{unknown:x}"),
                        ));
                    }
                }
                if manifest.writer_feature_flags & FLAG_USE_V2_FORMAT_DEPRECATED != 0 {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Warning,
                        file,
                        "deprecated V2-format writer feature flag is set",
                    ));
                }
                if manifest.reader_feature_flags & FLAG_UNSTABLE_DATA_OVERLAY_FILES != 0
                    || manifest.writer_feature_flags & FLAG_UNSTABLE_DATA_OVERLAY_FILES != 0
                    || manifest
                        .fragments
                        .iter()
                        .any(|fragment| fragment.overlay_count > 0)
                {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Warning,
                        file,
                        "dataset uses unstable data-overlay metadata without a production compatibility guarantee",
                    ));
                }
                let has_deletions = manifest
                    .fragments
                    .iter()
                    .any(|fragment| fragment.deletion_file.is_some());
                if has_deletions
                    && (manifest.reader_feature_flags & FLAG_DELETION_FILES == 0
                        || manifest.writer_feature_flags & FLAG_DELETION_FILES == 0)
                {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Error,
                        file,
                        "deletion files are present without both deletion-file feature flags",
                    ));
                }
                if !manifest.config.is_empty()
                    && manifest.writer_feature_flags & FLAG_TABLE_CONFIG == 0
                {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Error,
                        file,
                        "table config is present without the table-config writer feature flag",
                    ));
                }
                if !manifest.base_paths.is_empty()
                    && (manifest.reader_feature_flags & FLAG_BASE_PATHS == 0
                        || manifest.writer_feature_flags & FLAG_BASE_PATHS == 0)
                {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Error,
                        file,
                        "base paths are present without both base-path feature flags",
                    ));
                }
                if manifest
                    .data_format
                    .as_ref()
                    .is_some_and(|format| format.version == "2.3")
                {
                    diagnostics.push(lance_diagnostic(
                        "AL205",
                        Severity::Warning,
                        file,
                        "Lance data format 2.3 is explicitly marked unstable",
                    ));
                }
                diagnostics
            })
            .collect()
    }
}

struct MissingReferences;

impl Rule for MissingReferences {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL206",
            name: "missing-lance-references",
            category: "correctness",
            default_severity: Severity::Error,
            summary: "Locally resolvable Lance manifest references must exist and match metadata.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, _, manifest)| {
                let root = PathBuf::from(&file.path);
                let mut diagnostics = Vec::new();
                let mut base_ids = BTreeSet::new();
                for base_path in &manifest.base_paths {
                    if !base_ids.insert(base_path.id) {
                        diagnostics.push(lance_diagnostic(
                            "AL206",
                            Severity::Error,
                            file,
                            format!("base path ID {} is duplicated", base_path.id),
                        ));
                    }
                    if base_path.path.is_empty() {
                        diagnostics.push(lance_diagnostic(
                            "AL206",
                            Severity::Error,
                            file,
                            format!("base path ID {} has an empty path", base_path.id),
                        ));
                    }
                }

                for fragment in &manifest.fragments {
                    for data_file in &fragment.files {
                        if let Some(base_id) = data_file.base_id {
                            if !base_ids.contains(&base_id) {
                                diagnostics.push(
                                    lance_diagnostic(
                                        "AL206",
                                        Severity::Error,
                                        file,
                                        format!(
                                            "data file `{}` references missing base path ID {base_id}",
                                            data_file.path
                                        ),
                                    )
                                    .with_location(format!("fragment={}", fragment.id)),
                                );
                            }
                            continue;
                        }
                        if !is_safe_relative_path(&data_file.path) {
                            continue;
                        }
                        let referenced_path = root.join("data").join(&data_file.path);
                        match fs::metadata(&referenced_path) {
                            Ok(file_metadata) => {
                                if data_file.file_size_bytes > 0
                                    && file_metadata.len() != data_file.file_size_bytes
                                {
                                    diagnostics.push(
                                        lance_diagnostic(
                                            "AL206",
                                            Severity::Error,
                                            file,
                                            format!(
                                                "data file `{}` records {} bytes but contains {} bytes",
                                                data_file.path,
                                                data_file.file_size_bytes,
                                                file_metadata.len()
                                            ),
                                        )
                                        .with_location(format!("fragment={}", fragment.id)),
                                    );
                                }
                            }
                            Err(_) => diagnostics.push(
                                lance_diagnostic(
                                    "AL206",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "referenced data file is missing: {}",
                                        referenced_path.display()
                                    ),
                                )
                                .with_location(format!("fragment={}", fragment.id)),
                            ),
                        }
                    }
                    if let Some(deletion) = &fragment.deletion_file {
                        if let Some(base_id) = deletion.base_id {
                            if !base_ids.contains(&base_id) {
                                diagnostics.push(
                                    lance_diagnostic(
                                        "AL206",
                                        Severity::Error,
                                        file,
                                        format!(
                                            "fragment {} deletion file references missing base path ID {base_id}",
                                            fragment.id
                                        ),
                                    )
                                    .with_location(format!("fragment={}", fragment.id)),
                                );
                            }
                        } else if let Some(extension) = deletion_extension(deletion.file_type) {
                            let referenced_path = root.join("_deletions").join(format!(
                                "{}-{}-{}.{}",
                                fragment.id, deletion.read_version, deletion.id, extension
                            ));
                            if !referenced_path.is_file() {
                                diagnostics.push(
                                    lance_diagnostic(
                                        "AL206",
                                        Severity::Error,
                                        file,
                                        format!(
                                            "referenced deletion file is missing: {}",
                                            referenced_path.display()
                                        ),
                                    )
                                    .with_location(format!("fragment={}", fragment.id)),
                                );
                            }
                        }
                    }
                }
                if !manifest.transaction_file.is_empty() {
                    if !is_safe_relative_path(&manifest.transaction_file) {
                        diagnostics.push(lance_diagnostic(
                            "AL206",
                            Severity::Error,
                            file,
                            format!(
                                "transaction file path is not a safe relative path: `{}`",
                                manifest.transaction_file
                            ),
                        ));
                    } else {
                        let transaction_path =
                            root.join("_transactions").join(&manifest.transaction_file);
                        if !transaction_path.is_file() {
                            diagnostics.push(lance_diagnostic(
                                "AL206",
                                Severity::Error,
                                file,
                                format!(
                                    "referenced transaction file is missing: {}",
                                    transaction_path.display()
                                ),
                            ));
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct MaintenanceOpportunities;

impl Rule for MaintenanceOpportunities {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL207",
            name: "lance-maintenance-opportunity",
            category: "maintenance",
            default_severity: Severity::Info,
            summary: "Lance histories, fragments, and deletions should be reviewed for compaction.",
        }
    }

    fn check(&self, dataset: &Dataset, config: &LintConfig) -> Vec<Diagnostic> {
        lance_manifests(dataset)
            .flat_map(|(file, metadata, manifest)| {
                let mut diagnostics = Vec::new();
                if config.rules.lance_max_versions > 0
                    && metadata.manifest_count > config.rules.lance_max_versions
                {
                    diagnostics.push(lance_diagnostic(
                        "AL207",
                        Severity::Info,
                        file,
                        format!(
                            "dataset retains {} versions, above configured review threshold {}",
                            metadata.manifest_count, config.rules.lance_max_versions
                        ),
                    ));
                }

                if config.rules.lance_target_fragment_rows > 0
                    && config.rules.lance_small_fragment_count > 0
                {
                    let small_fragments = manifest
                        .fragments
                        .iter()
                        .filter(|fragment| {
                            fragment.physical_rows > 0
                                && fragment.physical_rows
                                    < config.rules.lance_target_fragment_rows
                        })
                        .count();
                    if small_fragments >= config.rules.lance_small_fragment_count {
                        diagnostics.push(lance_diagnostic(
                            "AL207",
                            Severity::Info,
                            file,
                            format!(
                                "{small_fragments} fragments contain fewer than {} physical rows",
                                config.rules.lance_target_fragment_rows
                            ),
                        ));
                    }
                }

                if config.rules.lance_deletion_compaction_threshold > 0.0 {
                    for fragment in &manifest.fragments {
                        let Some(deletion) = &fragment.deletion_file else {
                            continue;
                        };
                        if fragment.physical_rows == 0 {
                            continue;
                        }
                        let ratio =
                            deletion.num_deleted_rows as f64 / fragment.physical_rows as f64;
                        if ratio > config.rules.lance_deletion_compaction_threshold {
                            diagnostics.push(
                                lance_diagnostic(
                                    "AL207",
                                    Severity::Info,
                                    file,
                                    format!(
                                        "fragment {} has {:.1}% deleted rows, above configured compaction threshold {:.1}%",
                                        fragment.id,
                                        ratio * 100.0,
                                        config.rules.lance_deletion_compaction_threshold * 100.0
                                    ),
                                )
                                .with_location(format!("fragment={}", fragment.id)),
                            );
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}

fn lance_files(dataset: &Dataset) -> impl Iterator<Item = (&DatasetFile, &LanceMetadata)> {
    dataset
        .files
        .iter()
        .filter(|file| file.format == Format::LanceDataset)
        .filter_map(|file| {
            file.lance_metadata
                .as_ref()
                .map(|metadata| (file, metadata))
        })
}

fn lance_manifests(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &LanceMetadata, &LanceManifest)> {
    lance_files(dataset).filter_map(|(file, metadata)| {
        metadata
            .manifest
            .as_ref()
            .map(|manifest| (file, metadata, manifest))
    })
}

fn lance_diagnostic(
    rule_id: &'static str,
    severity: Severity,
    file: &DatasetFile,
    message: impl Into<String>,
) -> Diagnostic {
    let help = match rule_id {
        "AL201" => "restore or regenerate the latest manifest with a compatible Lance writer",
        "AL202" => {
            "repair the dataset through Lance APIs so fragment IDs remain unique and monotonic"
        }
        "AL203" => "rewrite invalid fragment data through a compatible Lance implementation",
        "AL204" => "materialize or repair deletions through Lance APIs before reading the fragment",
        "AL205" => {
            "use a Lance release that supports every required feature before reading or writing"
        }
        "AL206" => {
            "restore referenced files or repair the dataset through Lance cleanup and recovery APIs"
        }
        _ => "review retention needs, then compact files or clean up old versions with Lance APIs",
    };
    let category = if rule_id == "AL207" {
        "maintenance"
    } else if rule_id == "AL205" {
        "interoperability"
    } else {
        "lance"
    };
    Diagnostic::new(rule_id, severity, category, message)
        .with_path(file.path.clone())
        .with_help(help)
}

fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn deletion_extension(file_type: i32) -> Option<&'static str> {
    match file_type {
        0 => Some("arrow"),
        1 => Some("bin"),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Message)]
struct PbManifest {
    #[prost(message, repeated, tag = "2")]
    fragments: Vec<PbFragment>,
    #[prost(uint64, tag = "3")]
    version: u64,
    #[prost(uint64, tag = "9")]
    reader_feature_flags: u64,
    #[prost(uint64, tag = "10")]
    writer_feature_flags: u64,
    #[prost(uint32, optional, tag = "11")]
    max_fragment_id: Option<u32>,
    #[prost(string, tag = "12")]
    transaction_file: String,
    #[prost(message, optional, tag = "15")]
    data_format: Option<PbDataStorageFormat>,
    #[prost(map = "string, string", tag = "16")]
    config: HashMap<String, String>,
    #[prost(message, repeated, tag = "18")]
    base_paths: Vec<PbBasePath>,
}

#[derive(Clone, PartialEq, Message)]
struct PbDataStorageFormat {
    #[prost(string, tag = "1")]
    file_format: String,
    #[prost(string, tag = "2")]
    version: String,
}

#[derive(Clone, PartialEq, Message)]
struct PbBasePath {
    #[prost(uint32, tag = "1")]
    id: u32,
    #[prost(bool, tag = "3")]
    is_dataset_root: bool,
    #[prost(string, tag = "4")]
    path: String,
}

#[derive(Clone, PartialEq, Message)]
struct PbFragment {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(message, repeated, tag = "2")]
    files: Vec<PbDataFile>,
    #[prost(message, optional, tag = "3")]
    deletion_file: Option<PbDeletionFile>,
    #[prost(uint64, tag = "4")]
    physical_rows: u64,
    #[prost(message, repeated, tag = "11")]
    overlays: Vec<PbDataOverlayFile>,
}

#[derive(Clone, PartialEq, Message)]
struct PbDataOverlayFile {}

#[derive(Clone, PartialEq, Message)]
struct PbDataFile {
    #[prost(string, tag = "1")]
    path: String,
    #[prost(int32, repeated, tag = "2")]
    fields: Vec<i32>,
    #[prost(int32, repeated, tag = "3")]
    column_indices: Vec<i32>,
    #[prost(uint32, tag = "4")]
    file_major_version: u32,
    #[prost(uint32, tag = "5")]
    file_minor_version: u32,
    #[prost(uint64, tag = "6")]
    file_size_bytes: u64,
    #[prost(uint32, optional, tag = "7")]
    base_id: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
struct PbDeletionFile {
    #[prost(int32, tag = "1")]
    file_type: i32,
    #[prost(uint64, tag = "2")]
    read_version: u64,
    #[prost(uint64, tag = "3")]
    id: u64,
    #[prost(uint64, tag = "4")]
    num_deleted_rows: u64,
    #[prost(uint32, optional, tag = "7")]
    base_id: Option<u32>,
}
