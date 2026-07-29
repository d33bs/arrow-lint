use std::{collections::HashMap, fs, path::Path};

use arrowlint_core::{lint_paths, LintConfig};
use prost::Message;
use tempfile::tempdir;

const FLAG_DELETION_FILES: u64 = 1;
const FLAG_BASE_PATHS: u64 = 16;
const FLAG_UNKNOWN: u64 = 128;

#[test]
fn scans_lance_v2_manifest_without_placeholder_diagnostic() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("vectors.lance");
    let manifest = valid_manifest();
    write_dataset(&root, &[(1, manifest)], NamingScheme::V2)?;

    let report = lint_paths(std::slice::from_ref(&root), LintConfig::default())?;
    let rule_ids = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(report.dataset.files.len(), 1);
    assert_eq!(report.dataset.files[0].format.as_str(), "lance");
    assert!(!rule_ids.contains(&"AL100"));
    assert!(
        !rule_ids.iter().any(|rule_id| rule_id.starts_with("AL2")),
        "unexpected Lance diagnostics: {rule_ids:?}"
    );
    Ok(())
}

#[test]
fn discovers_lance_dataset_once_from_parent_directory() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("vectors.lance");
    write_dataset(&root, &[(1, valid_manifest())], NamingScheme::V1)?;

    let report = lint_paths(&[directory.path().to_path_buf()], LintConfig::default())?;

    assert_eq!(report.dataset.files.len(), 1);
    assert_eq!(report.dataset.files[0].path, root.display().to_string());
    Ok(())
}

#[test]
fn reports_corrupt_and_mismatched_lance_manifests() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let corrupt_root = directory.path().join("corrupt.lance");
    fs::create_dir_all(corrupt_root.join("_versions"))?;
    fs::write(corrupt_root.join("_versions/1.manifest"), b"not a manifest")?;

    let corrupt_report = lint_paths(std::slice::from_ref(&corrupt_root), LintConfig::default())?;
    assert!(has_rule(&corrupt_report, "AL201"));

    let mismatch_root = directory.path().join("mismatch.lance");
    let mut manifest = valid_manifest();
    manifest.version = 2;
    write_dataset(&mismatch_root, &[(1, manifest)], NamingScheme::V1)?;

    let mismatch_report = lint_paths(std::slice::from_ref(&mismatch_root), LintConfig::default())?;
    assert!(has_rule(&mismatch_report, "AL201"));
    Ok(())
}

#[test]
fn reports_missing_versions_and_mixed_manifest_naming() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let missing_root = directory.path().join("missing-versions.lance");
    fs::create_dir_all(&missing_root)?;

    let missing_report = lint_paths(std::slice::from_ref(&missing_root), LintConfig::default())?;
    assert!(has_rule(&missing_report, "AL201"));

    let mixed_root = directory.path().join("mixed.lance");
    write_dataset(&mixed_root, &[(1, valid_manifest())], NamingScheme::V1)?;
    write_manifest(
        &mixed_root
            .join("_versions")
            .join(format!("{:020}.manifest", u64::MAX - 2)),
        &manifest_for_version(2),
    )?;

    let mixed_report = lint_paths(std::slice::from_ref(&mixed_root), LintConfig::default())?;
    assert!(has_rule(&mixed_report, "AL201"));
    Ok(())
}

#[test]
fn reports_invalid_fragment_and_data_file_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("invalid-fragments.lance");
    let mut manifest = valid_manifest();
    manifest.max_fragment_id = Some(0);
    manifest.fragments[0].files.push(TestDataFile {
        path: "second.lance".to_string(),
        fields: vec![0],
        column_indices: vec![1],
        file_major_version: 2,
        file_minor_version: 2,
        file_size_bytes: 7,
        base_id: None,
    });
    manifest.fragments.push(TestFragment {
        id: 0,
        files: vec![TestDataFile {
            path: "../outside.lance".to_string(),
            fields: vec![-1, 0],
            column_indices: vec![0, 0],
            file_major_version: 2,
            file_minor_version: 2,
            file_size_bytes: 0,
            base_id: None,
        }],
        deletion_file: None,
        physical_rows: 1,
    });
    write_dataset(&root, &[(1, manifest)], NamingScheme::V1)?;
    fs::write(root.join("data/second.lance"), b"fixture")?;

    let report = lint_paths(std::slice::from_ref(&root), LintConfig::default())?;

    assert!(has_rule(&report, "AL202"));
    assert!(has_rule(&report, "AL203"));
    Ok(())
}

#[test]
fn reports_invalid_deletions_and_feature_flags() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("deletions.lance");
    let mut manifest = valid_manifest();
    manifest.reader_feature_flags = FLAG_DELETION_FILES | FLAG_UNKNOWN;
    manifest.writer_feature_flags = FLAG_DELETION_FILES | FLAG_UNKNOWN;
    manifest.fragments[0].physical_rows = 1;
    manifest.fragments[0].deletion_file = Some(TestDeletionFile {
        file_type: 0,
        read_version: 2,
        id: 7,
        num_deleted_rows: 2,
        base_id: None,
    });
    write_dataset(&root, &[(1, manifest)], NamingScheme::V1)?;
    fs::create_dir_all(root.join("_deletions"))?;
    fs::write(root.join("_deletions/0-2-7.arrow"), b"fixture")?;

    let report = lint_paths(std::slice::from_ref(&root), LintConfig::default())?;

    assert!(has_rule(&report, "AL204"));
    assert!(has_rule(&report, "AL205"));
    Ok(())
}

#[test]
fn reports_missing_local_lance_references() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("missing-reference.lance");
    write_dataset(&root, &[(1, valid_manifest())], NamingScheme::V1)?;
    fs::remove_file(root.join("data/part.lance"))?;

    let report = lint_paths(std::slice::from_ref(&root), LintConfig::default())?;

    assert!(has_rule(&report, "AL206"));
    Ok(())
}

#[test]
fn reports_invalid_base_paths_and_known_size_mismatches() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("base-paths.lance");
    let mut manifest = valid_manifest();
    manifest.reader_feature_flags = FLAG_BASE_PATHS;
    manifest.writer_feature_flags = FLAG_BASE_PATHS;
    manifest.base_paths = vec![
        TestBasePath {
            id: 0,
            name: None,
            is_dataset_root: true,
            path: "/external/one".to_string(),
        },
        TestBasePath {
            id: 0,
            name: None,
            is_dataset_root: true,
            path: "/external/two".to_string(),
        },
    ];
    manifest.fragments[0].files.push(TestDataFile {
        path: "external.lance".to_string(),
        fields: vec![1],
        column_indices: vec![1],
        file_major_version: 2,
        file_minor_version: 2,
        file_size_bytes: 0,
        base_id: Some(7),
    });
    write_dataset(&root, &[(1, manifest)], NamingScheme::V1)?;
    fs::write(root.join("data/part.lance"), b"wrong-size")?;

    let report = lint_paths(std::slice::from_ref(&root), LintConfig::default())?;

    assert!(has_rule(&report, "AL206"));
    Ok(())
}

#[test]
fn reports_lance_maintenance_opportunities() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("history.lance");
    write_dataset(
        &root,
        &[
            (1, valid_manifest()),
            (2, manifest_for_version(2)),
            (3, manifest_for_version(3)),
        ],
        NamingScheme::V1,
    )?;
    let mut config = LintConfig::default();
    config.rules.lance_max_versions = 2;

    let report = lint_paths(std::slice::from_ref(&root), config)?;

    assert!(has_rule(&report, "AL207"));
    Ok(())
}

#[test]
fn reports_small_fragments_and_heavy_deletions() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let root = directory.path().join("fragment-maintenance.lance");
    let mut manifest = valid_manifest();
    manifest.reader_feature_flags = FLAG_DELETION_FILES;
    manifest.writer_feature_flags = FLAG_DELETION_FILES;
    manifest.fragments = (0..8)
        .map(|id| TestFragment {
            id,
            files: vec![TestDataFile {
                path: "part.lance".to_string(),
                fields: vec![0],
                column_indices: vec![0],
                file_major_version: 2,
                file_minor_version: 2,
                file_size_bytes: 7,
                base_id: None,
            }],
            deletion_file: Some(TestDeletionFile {
                file_type: 1,
                read_version: 1,
                id,
                num_deleted_rows: 20,
                base_id: None,
            }),
            physical_rows: 100,
        })
        .collect();
    manifest.max_fragment_id = Some(7);
    write_dataset(&root, &[(1, manifest)], NamingScheme::V1)?;
    fs::create_dir_all(root.join("_deletions"))?;
    for id in 0..8 {
        fs::write(root.join(format!("_deletions/{id}-1-{id}.bin")), b"fixture")?;
    }
    let mut config = LintConfig::default();
    config.rules.lance_max_versions = 0;
    config.rules.lance_target_fragment_rows = 1_000;
    config.rules.lance_small_fragment_count = 8;
    config.rules.lance_deletion_compaction_threshold = 0.1;

    let report = lint_paths(std::slice::from_ref(&root), config)?;
    let maintenance_count = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "AL207")
        .count();

    assert_eq!(maintenance_count, 9);
    Ok(())
}

fn has_rule(report: &arrowlint_core::LintReport, rule_id: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == rule_id)
}

fn manifest_for_version(version: u64) -> TestManifest {
    let mut manifest = valid_manifest();
    manifest.version = version;
    manifest
}

fn valid_manifest() -> TestManifest {
    TestManifest {
        fragments: vec![TestFragment {
            id: 0,
            files: vec![TestDataFile {
                path: "part.lance".to_string(),
                fields: vec![0],
                column_indices: vec![0],
                file_major_version: 2,
                file_minor_version: 2,
                file_size_bytes: 7,
                base_id: None,
            }],
            deletion_file: None,
            physical_rows: 1_048_576,
        }],
        version: 1,
        reader_feature_flags: 0,
        writer_feature_flags: 0,
        max_fragment_id: Some(0),
        transaction_file: String::new(),
        data_format: Some(TestDataStorageFormat {
            file_format: "lance".to_string(),
            version: "2.2".to_string(),
        }),
        config: HashMap::new(),
        base_paths: Vec::new(),
    }
}

fn write_dataset(
    root: &Path,
    manifests: &[(u64, TestManifest)],
    naming_scheme: NamingScheme,
) -> anyhow::Result<()> {
    fs::create_dir_all(root.join("_versions"))?;
    fs::create_dir_all(root.join("data"))?;
    fs::write(root.join("data/part.lance"), b"fixture")?;
    for (version, manifest) in manifests {
        let filename = match naming_scheme {
            NamingScheme::V1 => format!("{version}.manifest"),
            NamingScheme::V2 => format!("{:020}.manifest", u64::MAX - version),
        };
        write_manifest(&root.join("_versions").join(filename), manifest)?;
    }
    Ok(())
}

fn write_manifest(path: &Path, manifest: &TestManifest) -> anyhow::Result<()> {
    let message = manifest.encode_to_vec();
    let mut bytes = Vec::with_capacity(message.len() + 20);
    bytes.extend_from_slice(&(message.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&message);
    bytes.extend_from_slice(&0_i64.to_le_bytes());
    bytes.extend_from_slice(&0_i16.to_le_bytes());
    bytes.extend_from_slice(&1_i16.to_le_bytes());
    bytes.extend_from_slice(b"LANC");
    fs::write(path, bytes)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum NamingScheme {
    V1,
    V2,
}

#[derive(Clone, PartialEq, Message)]
struct TestManifest {
    #[prost(message, repeated, tag = "2")]
    fragments: Vec<TestFragment>,
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
    data_format: Option<TestDataStorageFormat>,
    #[prost(map = "string, string", tag = "16")]
    config: HashMap<String, String>,
    #[prost(message, repeated, tag = "18")]
    base_paths: Vec<TestBasePath>,
}

#[derive(Clone, PartialEq, Message)]
struct TestDataStorageFormat {
    #[prost(string, tag = "1")]
    file_format: String,
    #[prost(string, tag = "2")]
    version: String,
}

#[derive(Clone, PartialEq, Message)]
struct TestBasePath {
    #[prost(uint32, tag = "1")]
    id: u32,
    #[prost(string, optional, tag = "2")]
    name: Option<String>,
    #[prost(bool, tag = "3")]
    is_dataset_root: bool,
    #[prost(string, tag = "4")]
    path: String,
}

#[derive(Clone, PartialEq, Message)]
struct TestFragment {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(message, repeated, tag = "2")]
    files: Vec<TestDataFile>,
    #[prost(message, optional, tag = "3")]
    deletion_file: Option<TestDeletionFile>,
    #[prost(uint64, tag = "4")]
    physical_rows: u64,
}

#[derive(Clone, PartialEq, Message)]
struct TestDataFile {
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
struct TestDeletionFile {
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
