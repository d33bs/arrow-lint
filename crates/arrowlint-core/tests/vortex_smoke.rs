use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
};

use arrowlint_core::{lint_paths, LintConfig, Severity};
use flatbuffers::{FlatBufferBuilder, Push, PushAlignment, TableFinishedWIPOffset, WIPOffset};
use tempfile::tempdir;

type TableOffset = WIPOffset<TableFinishedWIPOffset>;

#[test]
fn scans_valid_vortex_file_without_placeholder_diagnostic() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("data.vx");
    write_vortex(&path, &FixtureOptions::default())?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;
    let rule_ids = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(report.dataset.files[0].format.as_str(), "vortex");
    assert!(!rule_ids.contains(&"AL100"));
    assert!(
        !rule_ids.iter().any(|rule_id| rule_id.starts_with("AL3")),
        "unexpected Vortex diagnostics: {rule_ids:?}"
    );
    Ok(())
}

#[test]
fn reports_invalid_vortex_envelope_and_version() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("invalid.vortex");
    write_vortex(&path, &FixtureOptions::default())?;
    let mut bytes = fs::read(&path)?;
    bytes[0..4].copy_from_slice(b"NOPE");
    let eof = bytes.len() - 8;
    bytes[eof..eof + 2].copy_from_slice(&2_u16.to_le_bytes());
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL301"));
    Ok(())
}

#[test]
fn reports_malformed_vortex_postscript() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("postscript.vortex");
    write_vortex(&path, &FixtureOptions::default())?;
    let mut bytes = fs::read(&path)?;
    let eof = bytes.len() - 8;
    let postscript_length = usize::from(u16::from_le_bytes([bytes[eof + 2], bytes[eof + 3]]));
    let postscript_start = eof - postscript_length;
    bytes[postscript_start..postscript_start + 4].fill(0);
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL301"));
    Ok(())
}

#[test]
fn reports_invalid_postscript_segments() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("segments.vortex");
    let options = FixtureOptions {
        layout: None,
        footer_locator: Segment {
            offset: u64::MAX,
            length: 8,
            alignment_exponent: 0,
            compression: None,
            encrypted: false,
        },
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL302"));
    Ok(())
}

#[test]
fn reports_malformed_vortex_footer() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("footer.vortex");
    write_vortex(&path, &FixtureOptions::default())?;
    let mut bytes = fs::read(&path)?;
    bytes[7..11].fill(0);
    fs::write(&path, bytes)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL304"));
    Ok(())
}

#[test]
fn reports_invalid_vortex_user_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("metadata.vortex");
    let options = FixtureOptions {
        metadata: vec![
            ("duplicate".to_string(), Segment::default()),
            ("duplicate".to_string(), Segment::default()),
            ("".to_string(), Segment::default()),
            ("x".repeat(65), Segment::default()),
        ],
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL303"));
    Ok(())
}

#[test]
fn skips_oversized_vortex_footer_without_allocating_it() -> anyhow::Result<()> {
    const OVERSIZED_FOOTER_LENGTH: u32 = 64 * 1024 * 1024 + 1;

    let directory = tempdir()?;
    let path = directory.path().join("oversized-footer.vortex");
    let options = FixtureOptions {
        footer_locator: Segment {
            offset: 7,
            length: OVERSIZED_FOOTER_LENGTH,
            ..Segment::default()
        },
        ..FixtureOptions::default()
    };
    let postscript = build_postscript(&options);
    let postscript_start = 7 + u64::from(OVERSIZED_FOOTER_LENGTH);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)?;
    file.write_all(b"VTXFDLS")?;
    file.seek(SeekFrom::Start(postscript_start))?;
    file.write_all(&postscript)?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&(postscript.len() as u16).to_le_bytes())?;
    file.write_all(b"VTXF")?;
    drop(file);

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.rule_id == "AL304" && diagnostic.message.contains("inspection limit")
        })
        .expect("oversized footer should be reported as skipped");

    assert_eq!(diagnostic.severity, Severity::Info);
    Ok(())
}

#[test]
fn reports_invalid_footer_segment_registry() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("footer-segments.vortex");
    let options = FixtureOptions {
        footer_segments: Some(vec![
            Segment {
                offset: 10,
                length: 1,
                alignment_exponent: 0,
                compression: None,
                encrypted: false,
            },
            Segment {
                offset: 4,
                length: u32::MAX,
                alignment_exponent: 64,
                compression: None,
                encrypted: false,
            },
        ]),
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL304"));
    Ok(())
}

#[test]
fn reports_invalid_vortex_registry_identifiers() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("registries.vortex");
    let options = FixtureOptions {
        array_ids: vec![
            "".to_string(),
            "vortex.primitive".to_string(),
            "vortex.primitive".to_string(),
        ],
        layout_ids: vec!["vortex.flat".to_string(), "vortex.flat".to_string()],
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL305"));
    Ok(())
}

#[test]
fn reports_unknown_vortex_compression() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("compression.vortex");
    let options = FixtureOptions {
        compression_schemes: vec![9],
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;

    assert!(has_rule(&report, "AL306"));
    Ok(())
}

#[test]
fn reports_missing_portability_and_pruning_metadata() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("minimal.vortex");
    let options = FixtureOptions {
        dtype: None,
        statistics: None,
        ..FixtureOptions::default()
    };
    write_vortex(&path, &options)?;

    let report = lint_paths(std::slice::from_ref(&path), LintConfig::default())?;
    let diagnostics = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "AL307")
        .count();

    assert_eq!(diagnostics, 2);
    Ok(())
}

fn has_rule(report: &arrowlint_core::LintReport, rule_id: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == rule_id)
}

#[derive(Clone)]
struct FixtureOptions {
    dtype: Option<Segment>,
    layout: Option<Segment>,
    statistics: Option<Segment>,
    footer_locator: Segment,
    metadata: Vec<(String, Segment)>,
    array_ids: Vec<String>,
    layout_ids: Vec<String>,
    footer_segments: Option<Vec<Segment>>,
    compression_schemes: Vec<u8>,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            dtype: Some(Segment {
                offset: 4,
                length: 1,
                ..Segment::default()
            }),
            layout: Some(Segment {
                offset: 5,
                length: 1,
                ..Segment::default()
            }),
            statistics: Some(Segment {
                offset: 6,
                length: 1,
                ..Segment::default()
            }),
            footer_locator: Segment::default(),
            metadata: Vec::new(),
            array_ids: vec!["vortex.primitive".to_string()],
            layout_ids: vec!["vortex.flat".to_string()],
            footer_segments: Some(vec![Segment {
                offset: 4,
                length: 3,
                ..Segment::default()
            }]),
            compression_schemes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Segment {
    offset: u64,
    length: u32,
    alignment_exponent: u8,
    compression: Option<u8>,
    encrypted: bool,
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct SegmentSpec([u8; 16]);

impl SegmentSpec {
    fn from_segment(segment: Segment) -> Self {
        let mut bytes = [0_u8; 16];
        bytes[0..8].copy_from_slice(&segment.offset.to_le_bytes());
        bytes[8..12].copy_from_slice(&segment.length.to_le_bytes());
        bytes[12] = segment.alignment_exponent;
        Self(bytes)
    }
}

impl Push for SegmentSpec {
    type Output = Self;

    unsafe fn push(&self, destination: &mut [u8], _written_len: usize) {
        destination.copy_from_slice(&self.0);
    }

    fn alignment() -> PushAlignment {
        PushAlignment::new(8)
    }
}

fn write_vortex(path: &Path, options: &FixtureOptions) -> anyhow::Result<()> {
    let footer = build_footer(options);
    let mut bytes = b"VTXF".to_vec();
    bytes.extend_from_slice(b"DLS");
    let footer_offset = bytes.len() as u64;
    bytes.extend_from_slice(&footer);

    let mut postscript_options = options.clone();
    if postscript_options.footer_locator.offset == 0
        && postscript_options.footer_locator.length == 0
    {
        postscript_options.footer_locator = Segment {
            offset: footer_offset,
            length: footer.len() as u32,
            ..Segment::default()
        };
    }
    let postscript = build_postscript(&postscript_options);
    bytes.extend_from_slice(&postscript);
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&(postscript.len() as u16).to_le_bytes());
    bytes.extend_from_slice(b"VTXF");
    fs::write(path, bytes)?;
    Ok(())
}

fn build_postscript(options: &FixtureOptions) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let dtype = options
        .dtype
        .map(|segment| build_postscript_segment(&mut builder, segment));
    let layout = options
        .layout
        .map(|segment| build_postscript_segment(&mut builder, segment));
    let statistics = options
        .statistics
        .map(|segment| build_postscript_segment(&mut builder, segment));
    let footer = build_postscript_segment(&mut builder, options.footer_locator);
    let metadata = options
        .metadata
        .iter()
        .map(|(key, segment)| {
            let key = builder.create_string(key);
            let segment = build_postscript_segment(&mut builder, *segment);
            let table = builder.start_table();
            builder.push_slot_always(4, key);
            builder.push_slot_always(6, segment);
            builder.end_table(table)
        })
        .collect::<Vec<_>>();
    let metadata = (!metadata.is_empty()).then(|| builder.create_vector(&metadata));

    let table = builder.start_table();
    if let Some(dtype) = dtype {
        builder.push_slot_always(4, dtype);
    }
    if let Some(layout) = layout {
        builder.push_slot_always(6, layout);
    }
    if let Some(statistics) = statistics {
        builder.push_slot_always(8, statistics);
    }
    builder.push_slot_always(10, footer);
    if let Some(metadata) = metadata {
        builder.push_slot_always(12, metadata);
    }
    let root = builder.end_table(table);
    builder.finish_minimal(root);
    builder.finished_data().to_vec()
}

fn build_postscript_segment(builder: &mut FlatBufferBuilder<'_>, segment: Segment) -> TableOffset {
    let compression = segment
        .compression
        .map(|scheme| build_compression(builder, scheme));
    let encryption = segment.encrypted.then(|| {
        let table = builder.start_table();
        builder.end_table(table)
    });
    let table = builder.start_table();
    builder.push_slot(4, segment.offset, 0);
    builder.push_slot(6, segment.length, 0);
    builder.push_slot(8, segment.alignment_exponent, 0);
    if let Some(compression) = compression {
        builder.push_slot_always(10, compression);
    }
    if let Some(encryption) = encryption {
        builder.push_slot_always(12, encryption);
    }
    builder.end_table(table)
}

fn build_footer(options: &FixtureOptions) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let array_ids = build_id_vector(&mut builder, &options.array_ids);
    let layout_ids = build_id_vector(&mut builder, &options.layout_ids);
    let segment_specs = options.footer_segments.as_ref().map(|segments| {
        let segments = segments
            .iter()
            .copied()
            .map(SegmentSpec::from_segment)
            .collect::<Vec<_>>();
        builder.create_vector(&segments)
    });
    let compression_specs = options
        .compression_schemes
        .iter()
        .copied()
        .map(|scheme| build_compression(&mut builder, scheme))
        .collect::<Vec<_>>();
    let compression_specs =
        (!compression_specs.is_empty()).then(|| builder.create_vector(&compression_specs));

    let table = builder.start_table();
    builder.push_slot_always(4, array_ids);
    builder.push_slot_always(6, layout_ids);
    if let Some(segment_specs) = segment_specs {
        builder.push_slot_always(8, segment_specs);
    }
    if let Some(compression_specs) = compression_specs {
        builder.push_slot_always(10, compression_specs);
    }
    let root = builder.end_table(table);
    builder.finish_minimal(root);
    builder.finished_data().to_vec()
}

fn build_id_vector<'builder>(
    builder: &mut FlatBufferBuilder<'builder>,
    identifiers: &[String],
) -> WIPOffset<flatbuffers::Vector<'builder, flatbuffers::ForwardsUOffset<TableFinishedWIPOffset>>>
{
    let tables = identifiers
        .iter()
        .map(|identifier| {
            let identifier = builder.create_string(identifier);
            let table = builder.start_table();
            builder.push_slot_always(4, identifier);
            builder.end_table(table)
        })
        .collect::<Vec<_>>();
    builder.create_vector(&tables)
}

fn build_compression(builder: &mut FlatBufferBuilder<'_>, scheme: u8) -> TableOffset {
    let table = builder.start_table();
    builder.push_slot(4, scheme, 0);
    builder.end_table(table)
}
