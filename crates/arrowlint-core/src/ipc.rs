use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result};
use arrow::ipc::{root_as_message, MessageHeader};

use crate::{
    dataset::{Dataset, Format, IpcKind, IpcMetadata},
    diagnostics::{Diagnostic, Severity},
    plugins::{Rule, RuleRegistry},
    rules::RuleMetadata,
    LintConfig,
};

const CONTINUATION_MARKER: [u8; 4] = [0xff; 4];
const MAX_MESSAGE_METADATA_BYTES: u32 = 64 * 1024 * 1024;

pub(crate) fn inspect_stream(path: &Path) -> Result<IpcMetadata> {
    let mut file =
        File::open(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    let file_size = file.metadata()?.len();
    inspect_stream_reader(&mut file, file_size)
}

pub(crate) fn inspect_stream_bytes(bytes: &[u8]) -> Result<IpcMetadata> {
    let mut cursor = Cursor::new(bytes);
    inspect_stream_reader(&mut cursor, bytes.len() as u64)
}

fn inspect_stream_reader(mut file: &mut impl ReadSeek, file_size: u64) -> Result<IpcMetadata> {
    let mut metadata = IpcMetadata {
        kind: IpcKind::Stream,
        errors: Vec::new(),
        uses_legacy_framing: false,
        has_eos: false,
        message_count: 0,
    };
    let mut offset = 0_u64;
    let mut schema_count = 0_usize;

    while offset < file_size {
        let frame_start = offset;
        let Some(prefix_end) = offset.checked_add(4) else {
            metadata
                .errors
                .push("IPC stream frame offset overflow".to_string());
            break;
        };
        if prefix_end > file_size {
            metadata
                .errors
                .push("IPC stream ends inside a frame prefix".to_string());
            break;
        }
        let prefix = read_array::<4>(&mut file, offset)?;
        offset = prefix_end;
        let continuation = prefix == CONTINUATION_MARKER;
        let metadata_length = if continuation {
            let Some(length_end) = offset.checked_add(4) else {
                metadata
                    .errors
                    .push("IPC stream metadata length offset overflow".to_string());
                break;
            };
            if length_end > file_size {
                metadata
                    .errors
                    .push("IPC stream ends after a continuation marker".to_string());
                break;
            }
            let bytes = read_array::<4>(&mut file, offset)?;
            offset = length_end;
            u32::from_le_bytes(bytes)
        } else {
            metadata.uses_legacy_framing = true;
            u32::from_le_bytes(prefix)
        };

        if metadata_length == 0 {
            metadata.has_eos = true;
            if offset != file_size {
                metadata
                    .errors
                    .push("IPC stream contains bytes after its EOS marker".to_string());
            }
            break;
        }
        if metadata_length > MAX_MESSAGE_METADATA_BYTES {
            metadata.errors.push(format!(
                "IPC message metadata length {metadata_length} exceeds the {MAX_MESSAGE_METADATA_BYTES}-byte inspection limit"
            ));
            break;
        }
        if continuation && metadata_length % 8 != 0 {
            metadata.errors.push(format!(
                "IPC message {} metadata length {metadata_length} is not 8-byte aligned",
                metadata.message_count
            ));
        }
        if !metadata.uses_legacy_framing && frame_start % 8 != 0 {
            metadata.errors.push(format!(
                "IPC message {} starts at unaligned offset {frame_start}",
                metadata.message_count
            ));
        }

        let Some(metadata_end) = offset.checked_add(u64::from(metadata_length)) else {
            metadata
                .errors
                .push("IPC message metadata range overflow".to_string());
            break;
        };
        if metadata_end > file_size {
            metadata.errors.push(format!(
                "IPC message {} metadata extends past the end of the stream",
                metadata.message_count
            ));
            break;
        }
        let message_bytes = read_bytes(&mut file, offset, metadata_length as usize)?;
        offset = metadata_end;
        let message = match root_as_message(&message_bytes) {
            Ok(message) => message,
            Err(error) => {
                metadata.errors.push(format!(
                    "IPC message {} has invalid FlatBuffer metadata: {error}",
                    metadata.message_count
                ));
                break;
            }
        };

        let body_length = message.bodyLength();
        if body_length < 0 {
            metadata.errors.push(format!(
                "IPC message {} has negative body length {body_length}",
                metadata.message_count
            ));
            break;
        }
        if body_length % 8 != 0 {
            metadata.errors.push(format!(
                "IPC message {} body length {body_length} is not a multiple of 8",
                metadata.message_count
            ));
        }
        let header = message.header_type();
        match header {
            MessageHeader::Schema => {
                if metadata.message_count != 0 {
                    metadata
                        .errors
                        .push("IPC stream contains a schema after the first message".to_string());
                }
                schema_count += 1;
                if body_length != 0 {
                    metadata
                        .errors
                        .push("IPC schema message must not have a body".to_string());
                }
            }
            MessageHeader::DictionaryBatch | MessageHeader::RecordBatch => {
                if schema_count == 0 {
                    metadata
                        .errors
                        .push("IPC stream data appears before its schema".to_string());
                }
            }
            _ => metadata.errors.push(format!(
                "IPC stream message {} uses unsupported header type {header:?}",
                metadata.message_count
            )),
        }

        let Some(body_end) = offset.checked_add(body_length as u64) else {
            metadata
                .errors
                .push("IPC message body range overflow".to_string());
            break;
        };
        if body_end > file_size {
            metadata.errors.push(format!(
                "IPC message {} body extends past the end of the stream",
                metadata.message_count
            ));
            break;
        }
        file.seek(SeekFrom::Start(body_end))?;
        offset = body_end;
        metadata.message_count += 1;
    }

    if schema_count == 0 {
        metadata
            .errors
            .push("IPC stream does not contain a schema message".to_string());
    } else if schema_count > 1 {
        metadata
            .errors
            .push("IPC stream contains more than one schema message".to_string());
    }
    Ok(metadata)
}

fn read_array<const LENGTH: usize>(file: &mut impl ReadSeek, offset: u64) -> Result<[u8; LENGTH]> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0_u8; LENGTH];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_bytes(file: &mut impl ReadSeek, offset: u64, length: usize) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

pub(crate) fn register_rules(registry: &mut RuleRegistry) {
    registry.register(InvalidIpcStream);
    registry.register(LegacyIpcStreamFraming);
}

struct InvalidIpcStream;

impl Rule for InvalidIpcStream {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL016",
            name: "invalid-arrow-ipc-stream",
            category: "correctness",
            default_severity: Severity::Error,
            summary: "Arrow IPC streams must have valid framing, metadata, and message ordering.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::ArrowIpc)
            .filter_map(|file| {
                file.ipc_metadata
                    .as_ref()
                    .filter(|metadata| metadata.kind == IpcKind::Stream)
                    .map(|metadata| (file, metadata))
            })
            .flat_map(|(file, metadata)| {
                metadata.errors.iter().map(|error| {
                    Diagnostic::new("AL016", Severity::Error, "correctness", error)
                        .with_path(file.path.clone())
                        .with_help(
                            "rewrite the stream with a current Arrow IPC writer and verify it with an independent reader",
                        )
                })
            })
            .collect()
    }
}

struct LegacyIpcStreamFraming;

impl Rule for LegacyIpcStreamFraming {
    fn metadata(&self) -> RuleMetadata {
        RuleMetadata {
            id: "AL017",
            name: "legacy-arrow-ipc-framing",
            category: "interoperability",
            default_severity: Severity::Warning,
            summary: "Stored Arrow IPC streams should use continuation-marker framing.",
        }
    }

    fn check(&self, dataset: &Dataset, _config: &LintConfig) -> Vec<Diagnostic> {
        dataset
            .files
            .iter()
            .filter(|file| file.format == Format::ArrowIpc)
            .filter_map(|file| {
                file.ipc_metadata
                    .as_ref()
                    .filter(|metadata| {
                        metadata.kind == IpcKind::Stream && metadata.uses_legacy_framing
                    })
                    .map(|_| {
                        Diagnostic::new(
                            "AL017",
                            Severity::Warning,
                            "interoperability",
                            "IPC stream uses legacy framing without a continuation marker",
                        )
                        .with_path(file.path.clone())
                        .with_help(
                            "rewrite stored streams with a current Arrow writer to preserve 8-byte message alignment",
                        )
                    })
            })
            .collect()
    }
}
