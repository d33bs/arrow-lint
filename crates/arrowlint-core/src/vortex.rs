use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result};

use crate::{
    dataset::{
        Dataset, DatasetFile, Format, VortexFooter, VortexFooterSegment, VortexMetadata,
        VortexPostscript, VortexSegment, VortexUserMetadata,
    },
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    rules::RuleMetadata,
    LintConfig,
};

const MAGIC: &[u8; 4] = b"VTXF";
const VERSION: u16 = 1;
const EOF_SIZE: u64 = 8;
const MAX_POSTSCRIPT_SIZE: u16 = u16::MAX - 8;
const MAX_METADATA_SEGMENTS: usize = 16;
const MAX_METADATA_KEY_BYTES: usize = 64;
const MAX_FOOTER_INSPECTION_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn scan_file(path: &Path) -> Result<DatasetFile> {
    let metadata = inspect_file(path)?;
    let mut summary = BTreeMap::new();
    if let Some(version) = metadata.version {
        summary.insert("vortex.version".to_string(), version.to_string());
    }
    if let Some(postscript) = &metadata.postscript {
        summary.insert(
            "vortex.user_metadata_segments".to_string(),
            postscript.metadata.len().to_string(),
        );
    }
    if let Some(footer) = &metadata.footer {
        summary.insert(
            "vortex.segments".to_string(),
            footer
                .segment_specs
                .as_ref()
                .map_or(0, Vec::len)
                .to_string(),
        );
    }

    Ok(DatasetFile {
        path: path.display().to_string(),
        format: Format::Vortex,
        size_bytes: metadata.file_size,
        num_rows: None,
        schema: None,
        metadata: summary,
        row_groups: Vec::new(),
        iceberg_metadata: None,
        lance_metadata: None,
        vortex_metadata: Some(metadata),
        ipc_metadata: None,
        geoparquet_metadata: None,
    })
}

fn inspect_file(path: &Path) -> Result<VortexMetadata> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let file_size = file.metadata()?.len();
    let mut metadata = VortexMetadata {
        file_size,
        ..VortexMetadata::default()
    };
    if file_size < 12 {
        metadata
            .errors
            .push("file is smaller than the minimum Vortex container".to_string());
        return Ok(metadata);
    }

    let start = read_range(&mut file, 0, 4)?;
    if start.as_slice() != MAGIC {
        metadata
            .errors
            .push("leading magic bytes are not `VTXF`".to_string());
    }
    let eof = read_range(&mut file, file_size - EOF_SIZE, EOF_SIZE as usize)?;
    if &eof[4..8] != MAGIC {
        metadata
            .errors
            .push("trailing magic bytes are not `VTXF`".to_string());
    }

    let version = u16::from_le_bytes([eof[0], eof[1]]);
    metadata.version = Some(version);
    if version != VERSION {
        metadata.errors.push(format!(
            "unsupported Vortex file version {version}; expected {VERSION}"
        ));
    }
    let postscript_length = u16::from_le_bytes([eof[2], eof[3]]);
    metadata.postscript_length = Some(postscript_length);
    if postscript_length > MAX_POSTSCRIPT_SIZE {
        metadata.errors.push(format!(
            "postscript length {postscript_length} exceeds format maximum {MAX_POSTSCRIPT_SIZE}"
        ));
    }
    let Some(postscript_start) = file_size
        .checked_sub(EOF_SIZE)
        .and_then(|offset| offset.checked_sub(u64::from(postscript_length)))
    else {
        metadata
            .errors
            .push("postscript length extends before the start of the file".to_string());
        return Ok(metadata);
    };
    metadata.postscript_start = Some(postscript_start);
    if postscript_start < 4 {
        metadata
            .errors
            .push("postscript overlaps the leading file magic".to_string());
        return Ok(metadata);
    }

    let postscript_bytes = read_range(&mut file, postscript_start, usize::from(postscript_length))?;
    let postscript = match parse_postscript(&postscript_bytes) {
        Ok(postscript) => postscript,
        Err(error) => {
            metadata
                .errors
                .push(format!("invalid postscript FlatBuffer: {error:#}"));
            return Ok(metadata);
        }
    };

    if let Some(footer_segment) = &postscript.footer {
        if footer_segment.compression_scheme.is_none()
            && !footer_segment.encrypted
            && segment_range_is_readable(footer_segment, file_size)
        {
            let footer_length = usize::try_from(footer_segment.length)?;
            if footer_length > MAX_FOOTER_INSPECTION_BYTES {
                metadata.footer_skip_reason = Some(format!(
                    "footer is {footer_length} bytes, exceeding the {}-byte inspection limit",
                    MAX_FOOTER_INSPECTION_BYTES
                ));
            } else {
                let footer_bytes = read_range(&mut file, footer_segment.offset, footer_length)?;
                match parse_footer(&footer_bytes) {
                    Ok(footer) => metadata.footer = Some(footer),
                    Err(error) => {
                        metadata.footer_error =
                            Some(format!("invalid footer FlatBuffer: {error:#}"));
                    }
                }
            }
        }
    }
    metadata.postscript = Some(postscript);
    Ok(metadata)
}

fn read_range(file: &mut File, offset: u64, length: usize) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn parse_postscript(bytes: &[u8]) -> Result<VortexPostscript> {
    let root = FlatTable::root(bytes)?;
    let metadata = match root.vector(4, 4)? {
        Some(vector) => (0..vector.len)
            .map(|index| {
                let entry = vector.table(index)?;
                Ok(VortexUserMetadata {
                    key: entry.string(0)?,
                    segment: entry.table(1)?.map(parse_postscript_segment).transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(VortexPostscript {
        dtype: root.table(0)?.map(parse_postscript_segment).transpose()?,
        layout: root.table(1)?.map(parse_postscript_segment).transpose()?,
        statistics: root.table(2)?.map(parse_postscript_segment).transpose()?,
        footer: root.table(3)?.map(parse_postscript_segment).transpose()?,
        metadata,
    })
}

fn parse_postscript_segment(table: FlatTable<'_>) -> Result<VortexSegment> {
    Ok(VortexSegment {
        offset: table.u64(0, 0)?,
        length: table.u32(1, 0)?,
        alignment_exponent: table.u8(2, 0)?,
        compression_scheme: table.table(3)?.map(|table| table.u8(0, 0)).transpose()?,
        encrypted: table.table(4)?.is_some(),
    })
}

fn parse_footer(bytes: &[u8]) -> Result<VortexFooter> {
    let root = FlatTable::root(bytes)?;
    let array_ids = parse_identifier_vector(&root, 0)?;
    let layout_ids = parse_identifier_vector(&root, 1)?;
    let segment_specs = root
        .vector(2, 16)?
        .map(|vector| {
            (0..vector.len)
                .map(|index| {
                    let bytes = vector.element(index)?;
                    Ok(VortexFooterSegment {
                        offset: read_u64(bytes, 0)?,
                        length: read_u32(bytes, 8)?,
                        alignment_exponent: read_u8(bytes, 12)?,
                        compression_index: read_u8(bytes, 13)?,
                        encryption_index: read_u16(bytes, 14)?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let compression_schemes = match root.vector(3, 4)? {
        Some(vector) => (0..vector.len)
            .map(|index| vector.table(index)?.u8(0, 0))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let encryption_count = root.vector(4, 4)?.map_or(0, |vector| vector.len);
    Ok(VortexFooter {
        array_ids,
        layout_ids,
        segment_specs,
        compression_schemes,
        encryption_count,
    })
}

fn parse_identifier_vector(root: &FlatTable<'_>, slot: usize) -> Result<Vec<Option<String>>> {
    match root.vector(slot, 4)? {
        Some(vector) => (0..vector.len)
            .map(|index| vector.table(index)?.string(0))
            .collect(),
        None => Ok(Vec::new()),
    }
}

#[derive(Clone, Copy)]
struct FlatTable<'a> {
    bytes: &'a [u8],
    position: usize,
    object_end: usize,
    vtable_position: usize,
    vtable_length: usize,
}

impl<'a> FlatTable<'a> {
    fn root(bytes: &'a [u8]) -> Result<Self> {
        let position = usize::try_from(read_u32(bytes, 0)?)?;
        Self::at(bytes, position)
    }

    fn at(bytes: &'a [u8], position: usize) -> Result<Self> {
        let vtable_offset = i64::from(read_i32(bytes, position)?);
        let vtable_position = i64::try_from(position)?
            .checked_sub(vtable_offset)
            .and_then(|position| usize::try_from(position).ok())
            .context("FlatBuffer vtable offset is outside the buffer")?;
        let vtable_length = usize::from(read_u16(bytes, vtable_position)?);
        let object_length = usize::from(read_u16(bytes, vtable_position + 2)?);
        if vtable_length < 4 || vtable_length % 2 != 0 {
            anyhow::bail!("FlatBuffer vtable has invalid length {vtable_length}");
        }
        let vtable_end = vtable_position
            .checked_add(vtable_length)
            .context("FlatBuffer vtable length overflow")?;
        if vtable_end > bytes.len() {
            anyhow::bail!("FlatBuffer vtable extends past the buffer");
        }
        let object_end = position
            .checked_add(object_length)
            .context("FlatBuffer table length overflow")?;
        if object_length < 4 || object_end > bytes.len() {
            anyhow::bail!("FlatBuffer table extends past the buffer");
        }
        Ok(Self {
            bytes,
            position,
            object_end,
            vtable_position,
            vtable_length,
        })
    }

    fn field(&self, slot: usize) -> Result<Option<usize>> {
        let entry = 4_usize
            .checked_add(slot.checked_mul(2).context("FlatBuffer slot overflow")?)
            .context("FlatBuffer slot overflow")?;
        let entry_end = entry
            .checked_add(2)
            .context("FlatBuffer vtable entry overflow")?;
        if entry_end > self.vtable_length {
            return Ok(None);
        }
        let offset = usize::from(read_u16(self.bytes, self.vtable_position + entry)?);
        if offset == 0 {
            return Ok(None);
        }
        let position = self
            .position
            .checked_add(offset)
            .context("FlatBuffer field offset overflow")?;
        if position >= self.object_end {
            anyhow::bail!("FlatBuffer field lies outside its table");
        }
        Ok(Some(position))
    }

    fn table(&self, slot: usize) -> Result<Option<Self>> {
        self.field(slot)?
            .map(|field| {
                indirect(self.bytes, field).and_then(|position| Self::at(self.bytes, position))
            })
            .transpose()
    }

    fn string(&self, slot: usize) -> Result<Option<String>> {
        self.field(slot)?
            .map(|field| {
                let position = indirect(self.bytes, field)?;
                let length = usize::try_from(read_u32(self.bytes, position)?)?;
                let start = position
                    .checked_add(4)
                    .context("FlatBuffer string offset overflow")?;
                let end = start
                    .checked_add(length)
                    .context("FlatBuffer string length overflow")?;
                if end >= self.bytes.len() || self.bytes[end] != 0 {
                    anyhow::bail!("FlatBuffer string is out of bounds or not null terminated");
                }
                Ok(std::str::from_utf8(&self.bytes[start..end])?.to_string())
            })
            .transpose()
    }

    fn vector(&self, slot: usize, element_size: usize) -> Result<Option<FlatVector<'a>>> {
        self.field(slot)?
            .map(|field| {
                let position = indirect(self.bytes, field)?;
                let len = usize::try_from(read_u32(self.bytes, position)?)?;
                let data = position
                    .checked_add(4)
                    .context("FlatBuffer vector offset overflow")?;
                let byte_length = len
                    .checked_mul(element_size)
                    .context("FlatBuffer vector length overflow")?;
                let data_end = data
                    .checked_add(byte_length)
                    .context("FlatBuffer vector end overflow")?;
                if data_end > self.bytes.len() {
                    anyhow::bail!("FlatBuffer vector extends past the buffer");
                }
                Ok(FlatVector {
                    bytes: self.bytes,
                    data,
                    len,
                    element_size,
                })
            })
            .transpose()
    }

    fn u64(&self, slot: usize, default: u64) -> Result<u64> {
        self.field(slot)?
            .map_or(Ok(default), |position| read_u64(self.bytes, position))
    }

    fn u32(&self, slot: usize, default: u32) -> Result<u32> {
        self.field(slot)?
            .map_or(Ok(default), |position| read_u32(self.bytes, position))
    }

    fn u8(&self, slot: usize, default: u8) -> Result<u8> {
        self.field(slot)?
            .map_or(Ok(default), |position| read_u8(self.bytes, position))
    }
}

struct FlatVector<'a> {
    bytes: &'a [u8],
    data: usize,
    len: usize,
    element_size: usize,
}

impl<'a> FlatVector<'a> {
    fn element(&self, index: usize) -> Result<&'a [u8]> {
        if index >= self.len {
            anyhow::bail!("FlatBuffer vector index is out of bounds");
        }
        let start = self
            .data
            .checked_add(
                index
                    .checked_mul(self.element_size)
                    .context("FlatBuffer vector index overflow")?,
            )
            .context("FlatBuffer vector offset overflow")?;
        let end = start
            .checked_add(self.element_size)
            .context("FlatBuffer vector element overflow")?;
        self.bytes
            .get(start..end)
            .context("FlatBuffer vector element extends past the buffer")
    }

    fn table(&self, index: usize) -> Result<FlatTable<'a>> {
        if index >= self.len {
            anyhow::bail!("FlatBuffer table vector index is out of bounds");
        }
        let element = self
            .data
            .checked_add(
                index
                    .checked_mul(4)
                    .context("FlatBuffer table index overflow")?,
            )
            .context("FlatBuffer table vector offset overflow")?;
        FlatTable::at(self.bytes, indirect(self.bytes, element)?)
    }
}

fn indirect(bytes: &[u8], position: usize) -> Result<usize> {
    position
        .checked_add(usize::try_from(read_u32(bytes, position)?)?)
        .context("FlatBuffer indirect offset overflow")
}

fn read_u8(bytes: &[u8], position: usize) -> Result<u8> {
    bytes
        .get(position)
        .copied()
        .context("FlatBuffer scalar extends past the buffer")
}

fn read_u16(bytes: &[u8], position: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(read_array(bytes, position)?))
}

fn read_i32(bytes: &[u8], position: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(read_array(bytes, position)?))
}

fn read_u32(bytes: &[u8], position: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(read_array(bytes, position)?))
}

fn read_u64(bytes: &[u8], position: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(read_array(bytes, position)?))
}

fn read_array<const LENGTH: usize>(bytes: &[u8], position: usize) -> Result<[u8; LENGTH]> {
    let end = position
        .checked_add(LENGTH)
        .context("FlatBuffer scalar offset overflow")?;
    bytes
        .get(position..end)
        .context("FlatBuffer scalar extends past the buffer")?
        .try_into()
        .context("FlatBuffer scalar has the wrong length")
}

pub(crate) fn register_rules(registry: &mut RuleRegistry) {
    registry.register(InvalidContainer);
    registry.register(InvalidPostscriptSegments);
    registry.register(InvalidUserMetadata);
    registry.register(InvalidFooterSegments);
    registry.register(InvalidRegistries);
    registry.register(IncompatibleCompression);
    registry.register(MissingOptimizationMetadata);
}

struct InvalidContainer;

impl Rule for InvalidContainer {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL301",
            name: "invalid-vortex-container",
            category: "vortex",
            default_severity: Severity::Error,
            summary: "Vortex files must have a valid VTXF envelope and postscript.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_files(dataset)
            .flat_map(|(file, metadata)| {
                metadata
                    .errors
                    .iter()
                    .map(|error| vortex_diagnostic("AL301", Severity::Error, file, error))
            })
            .collect()
    }
}

struct InvalidPostscriptSegments;

impl Rule for InvalidPostscriptSegments {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL302",
            name: "invalid-vortex-postscript-segments",
            category: "vortex",
            default_severity: Severity::Error,
            summary: "Required Vortex postscript segments must be bounded and aligned.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_postscripts(dataset)
            .flat_map(|(file, metadata, postscript)| {
                let mut diagnostics = Vec::new();
                if postscript.layout.is_none() {
                    diagnostics.push(vortex_diagnostic(
                        "AL302",
                        Severity::Error,
                        file,
                        "postscript is missing the required layout segment",
                    ));
                }
                if postscript.footer.is_none() {
                    diagnostics.push(vortex_diagnostic(
                        "AL302",
                        Severity::Error,
                        file,
                        "postscript is missing the required footer segment",
                    ));
                }
                for (name, segment) in [
                    ("dtype", postscript.dtype.as_ref()),
                    ("layout", postscript.layout.as_ref()),
                    ("statistics", postscript.statistics.as_ref()),
                    ("footer", postscript.footer.as_ref()),
                ] {
                    if let Some(segment) = segment {
                        diagnostics.extend(segment_issues(
                            "AL302",
                            file,
                            name,
                            segment.offset,
                            segment.length,
                            segment.alignment_exponent,
                            metadata.postscript_start.unwrap_or(metadata.file_size),
                        ));
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidUserMetadata;

impl Rule for InvalidUserMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL303",
            name: "invalid-vortex-user-metadata",
            category: "vortex",
            default_severity: Severity::Error,
            summary: "Vortex user metadata keys and segment locators must satisfy format limits.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_postscripts(dataset)
            .flat_map(|(file, metadata, postscript)| {
                let mut diagnostics = Vec::new();
                if postscript.metadata.len() > MAX_METADATA_SEGMENTS {
                    diagnostics.push(vortex_diagnostic(
                        "AL303",
                        Severity::Error,
                        file,
                        format!(
                            "postscript contains {} user metadata segments; maximum is {MAX_METADATA_SEGMENTS}",
                            postscript.metadata.len()
                        ),
                    ));
                }
                let mut keys = BTreeSet::new();
                for (index, entry) in postscript.metadata.iter().enumerate() {
                    match entry.key.as_deref() {
                        None => diagnostics.push(
                            vortex_diagnostic(
                                "AL303",
                                Severity::Error,
                                file,
                                "user metadata entry is missing its required key",
                            )
                            .with_location(format!("metadata={index}")),
                        ),
                        Some("") => diagnostics.push(
                            vortex_diagnostic(
                                "AL303",
                                Severity::Error,
                                file,
                                "user metadata key must not be empty",
                            )
                            .with_location(format!("metadata={index}")),
                        ),
                        Some(key) if key.len() > MAX_METADATA_KEY_BYTES => diagnostics.push(
                            vortex_diagnostic(
                                "AL303",
                                Severity::Error,
                                file,
                                format!(
                                    "user metadata key is {} bytes; maximum is {MAX_METADATA_KEY_BYTES}",
                                    key.len()
                                ),
                            )
                            .with_location(format!("metadata={index}")),
                        ),
                        Some(key) if !keys.insert(key) => diagnostics.push(
                            vortex_diagnostic(
                                "AL303",
                                Severity::Error,
                                file,
                                format!("user metadata key `{key}` is duplicated"),
                            )
                            .with_location(format!("metadata={index}")),
                        ),
                        Some(_) => {}
                    }
                    match &entry.segment {
                        Some(segment) => diagnostics.extend(segment_issues(
                            "AL303",
                            file,
                            &format!("metadata[{index}]"),
                            segment.offset,
                            segment.length,
                            segment.alignment_exponent,
                            metadata.file_size,
                        )),
                        None => diagnostics.push(
                            vortex_diagnostic(
                                "AL303",
                                Severity::Error,
                                file,
                                "user metadata entry is missing its required segment",
                            )
                            .with_location(format!("metadata={index}")),
                        ),
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidFooterSegments;

impl Rule for InvalidFooterSegments {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL304",
            name: "invalid-vortex-footer-segments",
            category: "vortex",
            default_severity: Severity::Error,
            summary: "Vortex footer segment maps must be present, ordered, bounded, and aligned.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_files(dataset)
            .flat_map(|(file, metadata)| {
                let mut diagnostics = Vec::new();
                if let Some(error) = &metadata.footer_error {
                    diagnostics.push(vortex_diagnostic("AL304", Severity::Error, file, error));
                }
                if let Some(reason) = &metadata.footer_skip_reason {
                    diagnostics.push(
                        vortex_diagnostic("AL304", Severity::Info, file, reason).with_help(
                            "inspect this oversized footer with Vortex tooling; ArrowLint limits metadata reads to protect local and CI resources",
                        ),
                    );
                }
                let Some(footer) = &metadata.footer else {
                    return diagnostics;
                };
                let Some(segments) = &footer.segment_specs else {
                    diagnostics.push(vortex_diagnostic(
                        "AL304",
                        Severity::Error,
                        file,
                        "footer is missing the required segment registry",
                    ));
                    return diagnostics;
                };
                let mut previous_offset = None;
                for (index, segment) in segments.iter().enumerate() {
                    if previous_offset.is_some_and(|offset| segment.offset < offset) {
                        diagnostics.push(
                            vortex_diagnostic(
                                "AL304",
                                Severity::Error,
                                file,
                                "footer segment offsets are not ordered",
                            )
                            .with_location(format!("segment={index}")),
                        );
                    }
                    previous_offset = Some(segment.offset);
                    diagnostics.extend(segment_issues(
                        "AL304",
                        file,
                        &format!("segment[{index}]"),
                        segment.offset,
                        segment.length,
                        segment.alignment_exponent,
                        metadata.file_size,
                    ));
                }
                diagnostics
            })
            .collect()
    }
}

struct InvalidRegistries;

impl Rule for InvalidRegistries {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL305",
            name: "invalid-vortex-registries",
            category: "vortex",
            default_severity: Severity::Error,
            summary: "Vortex array and layout registry identifiers must be present and unique.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_footers(dataset)
            .flat_map(|(file, _, footer)| {
                let mut diagnostics = Vec::new();
                for (kind, identifiers) in
                    [("array", &footer.array_ids), ("layout", &footer.layout_ids)]
                {
                    let mut seen = BTreeSet::new();
                    for (index, identifier) in identifiers.iter().enumerate() {
                        match identifier.as_deref() {
                            None => diagnostics.push(
                                vortex_diagnostic(
                                    "AL305",
                                    Severity::Error,
                                    file,
                                    format!("{kind} registry entry is missing its required ID"),
                                )
                                .with_location(format!("{kind}_registry={index}")),
                            ),
                            Some("") => diagnostics.push(
                                vortex_diagnostic(
                                    "AL305",
                                    Severity::Error,
                                    file,
                                    format!("{kind} registry ID must not be empty"),
                                )
                                .with_location(format!("{kind}_registry={index}")),
                            ),
                            Some(identifier) if !seen.insert(identifier) => diagnostics.push(
                                vortex_diagnostic(
                                    "AL305",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "{kind} registry ID `{identifier}` appears more than once"
                                    ),
                                )
                                .with_location(format!("{kind}_registry={index}")),
                            ),
                            Some(_) => {}
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct IncompatibleCompression;

impl Rule for IncompatibleCompression {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL306",
            name: "incompatible-vortex-compression",
            category: "interoperability",
            default_severity: Severity::Error,
            summary: "Vortex compression registries must use known schemes within format limits.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_postscripts(dataset)
            .flat_map(|(file, metadata, postscript)| {
                let mut diagnostics = Vec::new();
                for (name, segment) in [
                    ("dtype", postscript.dtype.as_ref()),
                    ("layout", postscript.layout.as_ref()),
                    ("statistics", postscript.statistics.as_ref()),
                    ("footer", postscript.footer.as_ref()),
                ] {
                    if let Some(scheme) = segment.and_then(|segment| segment.compression_scheme) {
                        if scheme > 3 {
                            diagnostics.push(vortex_diagnostic(
                                "AL306",
                                Severity::Error,
                                file,
                                format!("{name} segment uses unknown compression scheme {scheme}"),
                            ));
                        }
                    }
                }
                if let Some(footer_segment) = &postscript.footer {
                    if metadata.footer.is_none()
                        && (footer_segment.compression_scheme.is_some()
                            || footer_segment.encrypted)
                    {
                        diagnostics.push(vortex_diagnostic(
                            "AL306",
                            Severity::Info,
                            file,
                            "footer is compressed or encrypted; deep footer registry checks were skipped",
                        ));
                    }
                }
                if let Some(footer) = &metadata.footer {
                    if footer.compression_schemes.len() > 8 {
                        diagnostics.push(vortex_diagnostic(
                            "AL306",
                            Severity::Error,
                            file,
                            format!(
                                "footer contains {} compression schemes; maximum is 8",
                                footer.compression_schemes.len()
                            ),
                        ));
                    }
                    for (index, scheme) in footer.compression_schemes.iter().enumerate() {
                        if *scheme > 3 {
                            diagnostics.push(
                                vortex_diagnostic(
                                    "AL306",
                                    Severity::Error,
                                    file,
                                    format!(
                                        "compression registry entry {index} uses unknown scheme {scheme}"
                                    ),
                                )
                                .with_location(format!("compression_registry={index}")),
                            );
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}

struct MissingOptimizationMetadata;

impl Rule for MissingOptimizationMetadata {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL307",
            name: "missing-vortex-optimization-metadata",
            category: "performance",
            default_severity: Severity::Info,
            summary: "Embedded dtypes and file statistics improve portability and pruning.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        vortex_postscripts(dataset)
            .flat_map(|(file, _, postscript)| {
                let mut diagnostics = Vec::new();
                if postscript.dtype.is_none() {
                    diagnostics.push(vortex_diagnostic(
                        "AL307",
                        Severity::Info,
                        file,
                        "file omits its dtype and requires an external dtype to open",
                    ));
                }
                if postscript.statistics.is_none() {
                    diagnostics.push(vortex_diagnostic(
                        "AL307",
                        Severity::Info,
                        file,
                        "file omits file-level statistics used for whole-file pruning",
                    ));
                }
                diagnostics
            })
            .collect()
    }
}

fn vortex_files(dataset: &Dataset) -> impl Iterator<Item = (&DatasetFile, &VortexMetadata)> {
    dataset
        .files
        .iter()
        .filter(|file| file.format == Format::Vortex)
        .filter_map(|file| {
            file.vortex_metadata
                .as_ref()
                .map(|metadata| (file, metadata))
        })
}

fn vortex_postscripts(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &VortexMetadata, &VortexPostscript)> {
    vortex_files(dataset).filter_map(|(file, metadata)| {
        metadata
            .postscript
            .as_ref()
            .map(|postscript| (file, metadata, postscript))
    })
}

fn vortex_footers(
    dataset: &Dataset,
) -> impl Iterator<Item = (&DatasetFile, &VortexMetadata, &VortexFooter)> {
    vortex_files(dataset).filter_map(|(file, metadata)| {
        metadata
            .footer
            .as_ref()
            .map(|footer| (file, metadata, footer))
    })
}

fn segment_issues(
    rule_id: &'static str,
    file: &DatasetFile,
    name: &str,
    offset: u64,
    length: u32,
    alignment_exponent: u8,
    boundary: u64,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if offset < 4 {
        diagnostics.push(vortex_diagnostic(
            rule_id,
            Severity::Error,
            file,
            format!("{name} segment starts before the leading file magic ends"),
        ));
    }
    let exceeds_boundary = match offset.checked_add(u64::from(length)) {
        Some(end) => end > boundary,
        None => true,
    };
    if exceeds_boundary {
        diagnostics.push(vortex_diagnostic(
            rule_id,
            Severity::Error,
            file,
            format!(
                "{name} segment at offset {offset} with length {length} exceeds byte boundary {boundary}"
            ),
        ));
    }
    if u32::from(alignment_exponent) >= usize::BITS {
        diagnostics.push(vortex_diagnostic(
            rule_id,
            Severity::Error,
            file,
            format!("{name} segment alignment exponent {alignment_exponent} is too large"),
        ));
    } else {
        let alignment = 1_u64 << alignment_exponent;
        if offset % alignment != 0 {
            diagnostics.push(vortex_diagnostic(
                rule_id,
                Severity::Error,
                file,
                format!("{name} segment offset {offset} is not aligned to {alignment} bytes"),
            ));
        }
    }
    diagnostics
}

fn segment_range_is_readable(segment: &VortexSegment, file_size: u64) -> bool {
    segment
        .offset
        .checked_add(u64::from(segment.length))
        .is_some_and(|end| end <= file_size)
}

fn vortex_diagnostic(
    rule_id: &'static str,
    severity: Severity,
    file: &DatasetFile,
    message: impl Into<String>,
) -> Diagnostic {
    let help = match rule_id {
        "AL301" => "rewrite the file with a stable Vortex writer; do not edit FlatBuffer offsets manually",
        "AL302" => "regenerate the file so required metadata segments are bounded and correctly aligned",
        "AL303" => "rewrite user metadata with unique UTF-8 keys no longer than 64 bytes",
        "AL304" => "regenerate the footer segment map with a compatible Vortex writer",
        "AL305" => "rewrite the file with unique, non-empty encoding and layout registry IDs",
        "AL306" => "use stable Vortex compression schemes supported by the declared file edition",
        _ => "embed the dtype and file-level statistics when portability and scan pruning are important",
    };
    let category = match rule_id {
        "AL306" => "interoperability",
        "AL307" => "performance",
        _ => "vortex",
    };
    Diagnostic::new(rule_id, severity, category, message)
        .with_path(file.path.clone())
        .with_help(help)
}
