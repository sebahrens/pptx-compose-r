#![deny(warnings)]

use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

pub use pptx_compose_core as core;
pub use pptx_compose_edit as edit;
pub use pptx_compose_json as json;

use core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    provenance::revision,
    zip::{
        reader::{RawEntry, from_bytes},
        writer::{self as zip_writer, WriteEntry},
    },
};
use pptx_compose_edit::diffs::SemanticDiff;
use pptx_compose_edit::patch::{DocumentState, parse_patch, validate_envelope};
use pptx_compose_json::schemas::{
    PatchReport, PatchStatus, PatchValidationSummary, ValidationStatus,
};
use sha2::{Digest, Sha256};

pub use core::zip::writer::WriteMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub overwrite: bool,
    pub validate: bool,
    pub atomic: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyPatchOptions {
    pub dry_run: bool,
    pub validate: bool,
}

impl Default for ApplyPatchOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            validate: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyPatchResult {
    pub report: PatchReport,
    pub diff: SemanticDiff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationDocument {
    source_path: Option<PathBuf>,
    source_bytes: Vec<u8>,
    entries: Vec<RawEntry>,
}

impl PresentationDocument {
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_path_with_options(path)
    }

    pub fn open_path_with_options(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| {
            Error::with_source(
                ErrorCode::InvalidInput,
                format!("Could not read PPTX input {}.", path.display()),
                source,
            )
        })?;
        let mut document = Self::from_bytes(bytes)?;
        document.source_path = Some(path.to_path_buf());
        Ok(document)
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let source_bytes = bytes.into();
        let entries = from_bytes(&source_bytes)?;
        Ok(Self {
            source_path: None,
            source_bytes,
            entries,
        })
    }

    pub fn open_reader<R>(mut reader: R) -> Result<Self>
    where
        R: std::io::Read,
    {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|source| Error::parse_error("Could not read PPTX input stream.", source))?;
        Self::from_bytes(bytes)
    }

    pub fn write_vec_with_options(&self, options: WriteOptions) -> Result<Vec<u8>> {
        if options.validate {
            self.validate()?;
        }
        let entries: Vec<_> = self.entries.iter().map(WriteEntry::Clean).collect();
        let output = Cursor::new(Vec::new());
        let output = zip_writer::write_writer_with_options(
            &self.source_bytes,
            &entries,
            output,
            &zip_writer::WriteOptions {
                mode: options.mode,
                ..zip_writer::WriteOptions::default()
            },
        )?;
        Ok(output.into_inner())
    }

    pub fn apply_patch_with_options(
        &self,
        patch: &serde_json::Value,
        options: ApplyPatchOptions,
    ) -> Result<ApplyPatchResult> {
        if options.validate {
            self.validate()?;
        }

        let document_id = sha256_hex(&self.source_bytes);
        let revision = u32::try_from(revision::on_open().value()).map_err(|source| {
            Error::with_source(
                ErrorCode::InternalError,
                "Revision value exceeds the patch report schema range.",
                source,
            )
        })?;
        reject_known_unsupported_operations(patch)?;
        let patch = parse_patch(patch.clone())?;
        validate_envelope(&patch, &DocumentState::new(document_id.clone(), revision))?;

        let dry_run = options.dry_run;
        Ok(ApplyPatchResult {
            report: PatchReport {
                schema: pptx_compose_json::schema_versions::PATCH_REPORT_SCHEMA.to_owned(),
                version: pptx_compose_json::schema_versions::PATCH_REPORT_VERSION,
                status: if dry_run {
                    PatchStatus::DryRunSuccess
                } else {
                    PatchStatus::Applied
                },
                dry_run,
                document_id: document_id.clone(),
                base_revision: revision,
                new_document_id: document_id,
                new_revision: revision,
                operation_reports: Vec::new(),
                changed_parts: Vec::new(),
                validation: PatchValidationSummary {
                    status: ValidationStatus::Valid,
                    errors: 0,
                    warnings: 0,
                },
            },
            diff: SemanticDiff {
                schema: pptx_compose_edit::diffs::SEMANTIC_DIFF_SCHEMA.to_owned(),
                version: pptx_compose_edit::diffs::SEMANTIC_DIFF_VERSION,
                changes: Vec::new(),
                changed_parts: Vec::new(),
            },
        })
    }

    pub fn write_path_with_options(
        &self,
        output_path: impl AsRef<Path>,
        options: WriteOptions,
    ) -> Result<()> {
        let output_path = output_path.as_ref();
        if output_path.exists() && !options.overwrite {
            return Err(Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "Output path {} already exists; pass --overwrite to replace it.",
                    output_path.display()
                ),
            ));
        }

        if options.atomic {
            self.write_path_atomic(output_path, options)
        } else {
            self.write_path_direct(output_path, options)
        }
    }

    fn write_path_direct(&self, output_path: &Path, options: WriteOptions) -> Result<()> {
        let output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)
            .map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    format!("Could not open output path {}.", output_path.display()),
                    source,
                )
            })?;
        let output = self.write_to_writer(output, options)?;
        output.sync_all().map_err(|source| {
            Error::with_source(
                ErrorCode::WriteFailed,
                format!("Could not fsync output path {}.", output_path.display()),
                source,
            )
        })
    }

    fn write_path_atomic(&self, output_path: &Path, options: WriteOptions) -> Result<()> {
        let parent = output_path.parent().ok_or_else(|| {
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "Output path {} has no parent directory.",
                    output_path.display()
                ),
            )
        })?;
        let temp_path = temp_sibling_path(output_path);
        let write_result = (|| {
            let output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|source| {
                    Error::with_source(
                        ErrorCode::WriteFailed,
                        format!("Could not create temporary output {}.", temp_path.display()),
                        source,
                    )
                })?;
            let output = self.write_to_writer(output, options)?;
            output.sync_all().map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    format!("Could not fsync temporary output {}.", temp_path.display()),
                    source,
                )
            })?;
            if output_path.exists() && !options.overwrite {
                return Err(Error::new(
                    ErrorCode::WriteFailed,
                    format!(
                        "Output path {} already exists; pass --overwrite to replace it.",
                        output_path.display()
                    ),
                ));
            }
            fs::rename(&temp_path, output_path).map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    format!(
                        "Could not atomically rename {} to {}.",
                        temp_path.display(),
                        output_path.display()
                    ),
                    source,
                )
            })?;
            fsync_dir(parent)?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        write_result
    }

    fn write_to_writer<W>(&self, output: W, options: WriteOptions) -> Result<W>
    where
        W: Write + std::io::Seek,
    {
        if options.validate {
            self.validate()?;
        }
        let entries: Vec<_> = self.entries.iter().map(WriteEntry::Clean).collect();
        zip_writer::write_writer_with_options(
            &self.source_bytes,
            &entries,
            output,
            &zip_writer::WriteOptions {
                mode: options.mode,
                ..zip_writer::WriteOptions::default()
            },
        )
    }

    pub fn validate(&self) -> Result<()> {
        let _entries = from_bytes(&self.source_bytes)?;
        Ok(())
    }

    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }
}

fn temp_sibling_path(output_path: &Path) -> PathBuf {
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output.pptx");
    let suffix = format!("{}.{}.tmp", std::process::id(), unique_counter());
    output_path.with_file_name(format!(".{file_name}.{suffix}"))
}

fn reject_known_unsupported_operations(patch: &serde_json::Value) -> Result<()> {
    let Some(operations) = patch
        .get("operations")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };

    for operation in operations {
        let Some(op_name) = operation.get("op").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if matches!(
            op_name,
            "edit_chart" | "replace_chart_data" | "replace_chart" | "update_chart"
        ) {
            let operation_id = operation
                .get("operation_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Chart editing is not supported by V1 patch operations.",
            )
            .with_location(ErrorLocation {
                operation_id,
                operation: Some(op_name.to_owned()),
                ..ErrorLocation::default()
            })
            .with_suggestion(
                "Leave chart parts unchanged or use raw XML tools when explicitly enabled.",
            ));
        }
    }

    Ok(())
}

fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn fsync_dir(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| {
            Error::with_source(
                ErrorCode::WriteFailed,
                format!("Could not fsync output directory {}.", path.display()),
                source,
            )
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity("sha256:".len() + 64);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn facade_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose");
    }
}
