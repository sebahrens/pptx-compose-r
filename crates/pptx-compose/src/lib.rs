#![deny(warnings)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions as FsOpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

mod options;

pub mod capabilities;

pub use options::{AgentViewOptions, ApplyPatchOptions, OpenOptions, WriteOptions};
pub use pptx_compose_core as core;
pub use pptx_compose_core::zip::writer::WriteMode;
pub use pptx_compose_edit as edit;
pub use pptx_compose_edit::{media_inputs::MediaInputs, patch::Patch};
pub use pptx_compose_json as json;

use core::{
    error::{Error, ErrorCode, Result},
    opc::{
        content_types::ContentTypes,
        package::Package,
        part::Part,
        part_name::PartName,
        relationships::{
            Relationship, RelationshipSet, RelationshipSource, TargetMode, relationships_to_xml,
        },
    },
    pptx::presentation as core_presentation,
    provenance::{
        checksum::part_checksum, document_id::document_id as provenance_document_id, revision,
    },
    xml::{document::XmlElement, parser::parse_document_with_limits},
    zip::{
        ZipEntryMetadata,
        limits::{OpenOptions as CoreOpenOptions, ResourceLimits},
        reader::{RawEntry, from_bytes_with_options, open_reader_with_options},
        sniff::sniff_package,
        writer::{self as zip_writer, DirtyEntry, PackageZipWriter, WriteEntry},
    },
};
use pptx_compose_edit::{
    diffs::{ChangedPart, DiffChange, PartChangeKind, SemanticDiff},
    media_inputs::MediaInputReport,
    operations::{
        ResolvedTarget,
        add_image::AddImage,
        add_text_box::AddTextBox,
        move_resize::MoveResize,
        replace_image::ReplaceImage,
        replace_text::{ReplaceNotesText, ReplaceTableCellText, ReplaceText},
        set_alt_text::SetAltText,
        set_document_metadata::SetDocumentMetadata,
    },
    patch::{
        ALL_OP_NAMES, DocumentState, Operation, OperationExecutor, PatchEffects,
        ValidationFailedReport, validate_envelope,
    },
    reports::has_blocking_findings,
    selectors::{self, Selector},
};
use pptx_compose_json::{
    agent_view::views::{FindTextRequest, ViewRequest},
    schemas::{PatchReport, PatchStatus, ValidationReport},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationDocument {
    source_path: Option<PathBuf>,
    source_bytes: Vec<u8>,
    entries: Vec<RawEntry>,
    resource_limits: ResourceLimits,
    dirty_parts: BTreeSet<PartName>,
    revision: revision::Revision,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaPartInfo {
    pub package_path: String,
    pub content_type: Option<String>,
    pub byte_length: u64,
    pub checksum: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyPatchOutput {
    pub report: PatchReport,
    pub diff: SemanticDiff,
}

impl PresentationDocument {
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_path_with_options(path, OpenOptions::default())
    }

    pub fn open_path_with_options(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).map_err(|source| {
            Error::with_source(
                ErrorCode::InvalidInput,
                format!("Could not read PPTX input metadata {}.", path.display()),
                source,
            )
        })?;
        ensure_facade_compressed_package_size(metadata.len(), options.resource_limits())?;
        let bytes = fs::read(path).map_err(|source| {
            Error::with_source(
                ErrorCode::InvalidInput,
                format!("Could not read PPTX input {}.", path.display()),
                source,
            )
        })?;
        let mut document = Self::from_bytes_with_options(bytes, options)?;
        document.source_path = Some(path.to_path_buf());
        Ok(document)
    }

    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self> {
        Self::from_bytes_with_options(bytes, OpenOptions::default())
    }

    pub fn from_bytes_with_options(bytes: impl AsRef<[u8]>, options: OpenOptions) -> Result<Self> {
        let source_bytes = bytes.as_ref().to_vec();
        sniff_package(&mut Cursor::new(source_bytes.as_slice()))?;
        let entries = from_bytes_with_options(
            &source_bytes,
            &CoreOpenOptions {
                resource_limits: options.resource_limits().clone(),
            },
        )?;
        package_from_entries_with_limits(&entries, options.resource_limits())?;
        Ok(Self {
            source_path: None,
            source_bytes,
            entries,
            resource_limits: options.resource_limits,
            dirty_parts: BTreeSet::new(),
            revision: revision::on_open(),
        })
    }

    pub fn open_reader<R>(mut reader: R) -> Result<Self>
    where
        R: std::io::Read,
    {
        Self::open_reader_with_options(&mut reader, OpenOptions::default())
    }

    pub fn open_reader_with_options<R>(mut reader: R, options: OpenOptions) -> Result<Self>
    where
        R: std::io::Read,
    {
        let bytes = read_compressed_input_with_limit(&mut reader, options.resource_limits())?;
        Self::from_bytes_with_options(bytes, options)
    }

    pub fn open_seek_reader<R>(mut reader: R) -> Result<Self>
    where
        R: std::io::Read + std::io::Seek,
    {
        Self::open_seek_reader_with_options(&mut reader, OpenOptions::default())
    }

    pub fn open_seek_reader_with_options<R>(mut reader: R, options: OpenOptions) -> Result<Self>
    where
        R: std::io::Read + std::io::Seek,
    {
        sniff_package(&mut reader)?;
        let entries = open_reader_with_options(
            &mut reader,
            &CoreOpenOptions {
                resource_limits: options.resource_limits().clone(),
            },
        )?;
        package_from_entries_with_limits(&entries, options.resource_limits())?;
        let mut source_bytes = Vec::new();
        reader
            .rewind()
            .map_err(|source| Error::parse_error("Could not rewind PPTX input stream.", source))?;
        reader
            .read_to_end(&mut source_bytes)
            .map_err(|source| Error::parse_error("Could not read PPTX input stream.", source))?;
        Ok(Self {
            source_path: None,
            source_bytes,
            entries,
            resource_limits: options.resource_limits,
            dirty_parts: BTreeSet::new(),
            revision: revision::on_open(),
        })
    }

    #[must_use]
    pub fn compressed_package_bytes(&self) -> u64 {
        u64::try_from(self.source_bytes.len()).unwrap_or(u64::MAX)
    }

    pub fn document_id(&self) -> Result<String> {
        document_id_from_entries(&self.entries)
    }

    #[must_use]
    pub fn slide_count(&self) -> u32 {
        self.entries
            .iter()
            .filter(|entry| !entry.meta.is_dir && is_slide_part_name(entry.name.as_str()))
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub fn presentation_slide_count(&self) -> Result<u32> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        let model = core_presentation::PresentationDocument::open(package)?;
        u32::try_from(model.slides().len()).map_err(|source| {
            Error::with_source(
                ErrorCode::ResourceLimitExceeded,
                "Presentation slide count exceeds the reportable range.",
                source,
            )
        })
    }

    pub fn to_agent_json(&self) -> Result<serde_json::Value> {
        self.to_agent_json_with_options(AgentViewOptions::default())
    }

    pub fn to_agent_json_with_options(
        &self,
        options: AgentViewOptions,
    ) -> Result<serde_json::Value> {
        self.to_agent_json_with_revision(options, self.revision.value())
    }

    pub fn to_agent_json_with_revision(
        &self,
        options: AgentViewOptions,
        revision: u64,
    ) -> Result<serde_json::Value> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        let model = core_presentation::PresentationDocument::open(package)?;
        let mut value = pptx_compose_json::agent_view::views::build_view_with_revision(
            &model,
            revision,
            ViewRequest {
                mode: options.mode,
                include_elements: options.include_elements,
                slide_id: options.slide_id,
                slide_ids: options.slide_ids,
                element_id: options.element_id,
                cursor: options.cursor,
                limit: options.limit,
            },
        )
        .map_err(json_error)?;
        normalize_agent_view_capabilities(&mut value);
        Ok(value)
    }

    pub fn to_legacy_json(&self) -> Result<serde_json::Value> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        pptx_compose_json::legacy_path_map::to_legacy_map(&package).map_err(json_error)
    }

    pub fn find_text(
        &self,
        request: FindTextRequest,
    ) -> Result<pptx_compose_json::agent_view::FindTextResult> {
        self.find_text_with_revision(request, self.revision.value())
    }

    pub fn find_text_with_revision(
        &self,
        request: FindTextRequest,
        revision: u64,
    ) -> Result<pptx_compose_json::agent_view::FindTextResult> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        let model = core_presentation::PresentationDocument::open(package)?;
        pptx_compose_json::agent_view::views::find_text_with_revision(&model, revision, request)
            .map_err(json_error)
    }

    pub fn from_legacy_json(value: serde_json::Value) -> Result<Self> {
        let package =
            pptx_compose_json::legacy_path_map::from_legacy_map(value).map_err(json_error)?;
        let bytes = package_to_zip_bytes(&package)?;
        Self::from_bytes(bytes)
    }

    pub fn write_vec_with_options(&self, options: WriteOptions) -> Result<Vec<u8>> {
        if options.validate {
            self.ensure_write_validation_passes()?;
        }
        let mut package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        apply_dirty_parts(&mut package, &self.dirty_parts);
        let output = Cursor::new(Vec::new());
        let output = write_package_to_writer(
            &self.source_bytes,
            &self.entries,
            &package,
            output,
            &zip_writer::WriteOptions {
                mode: options.mode,
                ..zip_writer::WriteOptions::default()
            },
        )?;
        Ok(output.into_inner())
    }

    pub fn write_vec(&self) -> Result<Vec<u8>> {
        self.write_vec_with_options(WriteOptions::default())
    }

    pub fn apply_patch(&mut self, patch: Patch, media: MediaInputs) -> Result<PatchReport> {
        self.apply_patch_with_options(patch, media, ApplyPatchOptions::default())
    }

    pub fn apply_patch_with_options(
        &mut self,
        patch: Patch,
        media: MediaInputs,
        options: ApplyPatchOptions,
    ) -> Result<PatchReport> {
        Ok(self.apply_patch_with_diff(patch, media, options)?.report)
    }

    pub fn apply_patch_with_diff(
        &mut self,
        patch: Patch,
        media: MediaInputs,
        options: ApplyPatchOptions,
    ) -> Result<ApplyPatchOutput> {
        if options.validate {
            let _report = self.validate()?;
        }

        let document_id = document_id_from_entries(&self.entries)?;
        let revision = self.current_revision()?;
        validate_envelope(&patch, &DocumentState::new(document_id.clone(), revision))?;
        let original_package =
            package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        let media_report = media.check_references(
            patch.operations.iter().filter_map(operation_media_ref),
            pptx_compose_edit::media_inputs::ExtraBindingPolicy::Warn,
        )?;

        let package = original_package.clone();
        let mut executor = RealOperationExecutor {
            media_inputs: &media,
        };
        let next_revision = next_revision_value(self.revision)?;
        let staged = pptx_compose_edit::patch::apply_patch_staged(
            &package,
            pptx_compose_edit::patch::PatchContext::new(
                document_id.clone(),
                revision,
                document_id.clone(),
                next_revision,
            ),
            &patch,
            options.dry_run,
            &mut executor,
        )?;
        let staged_package = staged.package;
        let staged_package = staged_package.package();
        let mut report = staged.report;
        append_media_input_warnings(&mut report, media_report);
        report.new_document_id = if report.changed_parts.is_empty() {
            document_id
        } else {
            document_id_from_package(staged_package, &self.entries)?
        };
        let diff = semantic_diff_for_patch(
            &original_package,
            staged_package,
            &self.entries,
            &patch.operations,
            &report,
        )?;
        if report.status == PatchStatus::Applied {
            self.replace_entries_from_package(staged_package)?;
        }
        if report.status == PatchStatus::Applied && !report.changed_parts.is_empty() {
            let recorded_revision =
                u32::try_from(self.revision.record_apply(true)).map_err(|source| {
                    Error::with_source(
                        ErrorCode::InternalError,
                        "Revision value exceeds the patch report schema range.",
                        source,
                    )
                })?;
            if recorded_revision != report.new_revision {
                return Err(Error::new(
                    ErrorCode::InternalError,
                    "Patch report revision does not match the recorded session revision.",
                ));
            }
        }
        Ok(ApplyPatchOutput { report, diff })
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
        let mut open_options = FsOpenOptions::new();
        open_options.write(true);
        if options.overwrite {
            open_options.create(true).truncate(true);
        } else {
            open_options.create_new(true);
        }
        let output = open_options.open(output_path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists && !options.overwrite {
                output_exists_error(output_path)
            } else {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    format!("Could not open output path {}.", output_path.display()),
                    source,
                )
            }
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
        let parent = output_parent(output_path);
        let temp_path = options
            .atomic_temp_path
            .clone()
            .unwrap_or_else(|| temp_output_path(output_path, None));
        let mut created_temp = false;
        let write_result = (|| {
            let output = FsOpenOptions::new()
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
            created_temp = true;
            let output = self.write_to_writer(output, options.clone())?;
            output.sync_all().map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    format!("Could not fsync temporary output {}.", temp_path.display()),
                    source,
                )
            })?;
            if options.overwrite {
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
            } else {
                fs::hard_link(&temp_path, output_path).map_err(|source| {
                    if source.kind() == std::io::ErrorKind::AlreadyExists {
                        output_exists_error(output_path)
                    } else {
                        Error::with_source(
                            ErrorCode::WriteFailed,
                            format!(
                                "Could not atomically publish {} to {} without replacing an existing file.",
                                temp_path.display(),
                                output_path.display()
                            ),
                            source,
                        )
                    }
                })?;
                fs::remove_file(&temp_path).map_err(|source| {
                    Error::with_source(
                        ErrorCode::WriteFailed,
                        format!("Could not remove temporary output {}.", temp_path.display()),
                        source,
                    )
                })?;
            }
            fsync_dir(parent)?;
            Ok(())
        })();

        if write_result.is_err() && created_temp && !options.keep_temp {
            let _ = fs::remove_file(&temp_path);
        }
        write_result
    }

    fn write_to_writer<W>(&self, output: W, options: WriteOptions) -> Result<W>
    where
        W: Write + std::io::Seek,
    {
        if options.validate {
            self.ensure_write_validation_passes()?;
        }
        let mut package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        apply_dirty_parts(&mut package, &self.dirty_parts);
        write_package_to_writer(
            &self.source_bytes,
            &self.entries,
            &package,
            output,
            &zip_writer::WriteOptions {
                mode: options.mode,
                ..zip_writer::WriteOptions::default()
            },
        )
    }

    pub fn validate(&self) -> Result<ValidationReport> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        pptx_compose_edit::reports::validation_report(
            core::validation::validate_package(&package, core::validation::ValidationMode::Edited),
            document_id_from_entries(&self.entries)?,
            self.current_revision()?,
        )
    }

    fn ensure_write_validation_passes(&self) -> Result<()> {
        let report = self.write_validation_report()?;
        if has_blocking_findings(&report) {
            return Err(Error::with_source(
                ErrorCode::ValidationFailed,
                "Package failed validation and was not written.",
                ValidationFailedReport { report },
            )
            .with_suggestion("Inspect the validation report, fix blocking findings, and retry."));
        }
        Ok(())
    }

    fn write_validation_report(&self) -> Result<ValidationReport> {
        let mut package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        apply_dirty_parts(&mut package, &self.dirty_parts);
        pptx_compose_edit::reports::validation_report(
            core::validation::validate_package(&package, core::validation::ValidationMode::Edited),
            document_id_from_entries(&self.entries)?,
            self.current_revision()?,
        )
    }

    pub fn media_parts(&self) -> Result<Vec<MediaPartInfo>> {
        let package = package_from_entries_with_limits(&self.entries, &self.resource_limits)?;
        let mut media = self
            .entries
            .iter()
            .filter(|entry| !entry.meta.is_dir && entry.name.as_str().starts_with("/ppt/media/"))
            .map(|entry| {
                u64::try_from(entry.bytes.len()).map_or_else(
                    |source| {
                        Err(Error::with_source(
                            ErrorCode::InternalError,
                            "Media part length exceeds the report schema range.",
                            source,
                        ))
                    },
                    |byte_length| {
                        Ok(MediaPartInfo {
                            package_path: entry.name.zip_entry_name().to_owned(),
                            content_type: package
                                .content_types()
                                .resolve(&entry.name)
                                .map(str::to_owned),
                            byte_length,
                            checksum: core::provenance::checksum::part_checksum(&entry.bytes),
                        })
                    },
                )
            })
            .collect::<Result<Vec<_>>>()?;
        media.sort_by(|left, right| left.package_path.cmp(&right.package_path));
        Ok(media)
    }

    pub fn media_part_bytes(&self, package_path: &str) -> Result<Vec<u8>> {
        let part_name = PartName::from_zip_entry(package_path)?;
        if !part_name.as_str().starts_with("/ppt/media/") {
            return Err(Error::unsafe_path(
                "Media package path must be under ppt/media/.",
            ));
        }
        self.entries
            .iter()
            .find(|entry| !entry.meta.is_dir && entry.name == part_name)
            .map(|entry| entry.bytes.clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidInput,
                    format!(
                        "Media package path {} was not found.",
                        part_name.zip_entry_name()
                    ),
                )
            })
    }

    pub fn write_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_path_with_options(path, WriteOptions::default())
    }

    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    fn current_revision(&self) -> Result<u32> {
        u32::try_from(self.revision.value()).map_err(|source| {
            Error::with_source(
                ErrorCode::InternalError,
                "Revision value exceeds the report schema range.",
                source,
            )
        })
    }

    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    fn replace_entries_from_package(&mut self, package: &Package) -> Result<()> {
        let package = package_with_serialized_control_parts(package, Some(&self.entries))?;
        let parts_by_name = package
            .parts()
            .iter()
            .map(|part| (part.name().clone(), part.bytes().to_vec()))
            .collect::<BTreeMap<_, _>>();

        for entry in &mut self.entries {
            if let Some(bytes) = parts_by_name.get(&entry.name) {
                entry.bytes.clone_from(bytes);
            }
        }

        for part in package.parts().iter() {
            if self.entries.iter().any(|entry| entry.name == *part.name()) {
                continue;
            }
            let entry_index = self.entries.len();
            self.entries.push(RawEntry {
                name: part.name().clone(),
                bytes: part.bytes().to_vec(),
                meta: dirty_zip_metadata(entry_index, part.original_zip_entry_name(), part.bytes()),
            });
        }

        self.dirty_parts = package.dirty_parts().clone();
        Ok(())
    }
}

fn semantic_diff_for_patch(
    before: &Package,
    after: &Package,
    source_entries: &[RawEntry],
    operations: &[Operation],
    report: &PatchReport,
) -> Result<SemanticDiff> {
    let before = package_with_serialized_control_parts(before, Some(source_entries))?;
    let after = package_with_serialized_control_parts(after, Some(source_entries))?;
    let changed_parts = report
        .changed_parts
        .iter()
        .map(|part| changed_part(&before, &after, part))
        .collect::<Result<Vec<_>>>()?;

    Ok(SemanticDiff {
        schema: pptx_compose_edit::diffs::SEMANTIC_DIFF_SCHEMA.to_owned(),
        version: pptx_compose_edit::diffs::SEMANTIC_DIFF_VERSION,
        changes: semantic_changes(operations, report),
        changed_parts,
    })
}

fn changed_part(before: &Package, after: &Package, part: &str) -> Result<ChangedPart> {
    let part_name = PartName::from_zip_entry(part)?;
    let before_part = before.parts().get(&part_name);
    let after_part = after.parts().get(&part_name);
    let after_bytes = after_part.map(|part| part.bytes()).unwrap_or_default();
    let change_kind = if before_part.is_none() {
        PartChangeKind::AddedPart
    } else {
        change_kind_for_part(part)
    };

    Ok(ChangedPart {
        part: part.to_owned(),
        change_kind,
        before_checksum: before_part
            .map(|part| part_checksum(part.bytes()))
            .unwrap_or_else(|| part_checksum(&[])),
        after_checksum: part_checksum(after_bytes),
    })
}

fn change_kind_for_part(part: &str) -> PartChangeKind {
    if part == "[Content_Types].xml" {
        PartChangeKind::ModifiedContentTypes
    } else if part.ends_with(".rels") && part.contains("/_rels/") {
        PartChangeKind::ModifiedRelationships
    } else if part.ends_with(".xml") {
        PartChangeKind::ModifiedXml
    } else {
        PartChangeKind::ModifiedBinary
    }
}

fn semantic_changes(operations: &[Operation], report: &PatchReport) -> Vec<DiffChange> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::ReplaceText(operation) => {
                let operation_report = report
                    .operation_reports
                    .iter()
                    .find(|report| report.operation_id == operation.operation_id)?;
                Some(DiffChange::TextReplaced {
                    operation_id: operation.operation_id.clone(),
                    element_id: operation_report.target.element_id.clone(),
                    before: serde_json::json!({ "text": operation.current_text_match }),
                    after: serde_json::json!({ "text": operation.text }),
                })
            }
            Operation::ReplaceNotesText(operation) => {
                let operation_report = report
                    .operation_reports
                    .iter()
                    .find(|report| report.operation_id == operation.operation_id)?;
                Some(DiffChange::TextReplaced {
                    operation_id: operation.operation_id.clone(),
                    element_id: operation_report.target.element_id.clone(),
                    before: serde_json::json!({ "text": operation.current_text_match }),
                    after: serde_json::json!({ "text": operation.text }),
                })
            }
            _ => None,
        })
        .collect()
}

#[must_use]
pub fn temp_output_path(output_path: &Path, temp_dir: Option<&Path>) -> PathBuf {
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output.pptx");
    let suffix = format!("{}.{}.tmp", std::process::id(), unique_counter());
    let temp_name = format!(".{file_name}.{suffix}");
    match temp_dir {
        Some(dir) => dir.join(temp_name),
        None => output_path.with_file_name(temp_name),
    }
}

fn output_exists_error(output_path: &Path) -> Error {
    Error::new(
        ErrorCode::WriteFailed,
        format!(
            "Output path {} already exists; pass --overwrite to replace it.",
            output_path.display()
        ),
    )
}

fn output_parent(output_path: &Path) -> &Path {
    match output_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
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

fn json_error(error: pptx_compose_json::schemas::JsonError) -> Error {
    match error {
        pptx_compose_json::schemas::JsonError::SerializeSchema(message)
        | pptx_compose_json::schemas::JsonError::InvalidCursor(message)
        | pptx_compose_json::schemas::JsonError::MalformedLegacyEnvelope(message)
        | pptx_compose_json::schemas::JsonError::Projection(message) => {
            Error::new(ErrorCode::InvalidInput, message)
        }
        pptx_compose_json::schemas::JsonError::ResourceLimitExceeded(message) => {
            Error::resource_limit_exceeded(message)
        }
        pptx_compose_json::schemas::JsonError::Core(error) => error,
        pptx_compose_json::schemas::JsonError::NotFound { kind, id } => Error::new(
            ErrorCode::SelectorNotFound,
            format!("{kind} `{id}` was not found."),
        ),
    }
}

fn read_compressed_input_with_limit<R>(
    reader: &mut R,
    resource_limits: &ResourceLimits,
) -> Result<Vec<u8>>
where
    R: Read,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| Error::parse_error("Could not read PPTX input stream.", source))?;
        if count == 0 {
            break;
        }
        let next_len = bytes
            .len()
            .checked_add(count)
            .and_then(|len| u64::try_from(len).ok())
            .unwrap_or(u64::MAX);
        ensure_facade_compressed_package_size(next_len, resource_limits)?;
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn ensure_facade_compressed_package_size(
    compressed_package_bytes: u64,
    resource_limits: &ResourceLimits,
) -> Result<()> {
    if compressed_package_bytes > resource_limits.max_compressed_package_bytes {
        return Err(Error::resource_limit_exceeded(format!(
            "ZIP package exceeded the maximum compressed size of {} bytes.",
            resource_limits.max_compressed_package_bytes
        )));
    }
    Ok(())
}

fn package_from_entries_with_limits(
    entries: &[RawEntry],
    resource_limits: &ResourceLimits,
) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        if entry.meta.is_dir {
            continue;
        }
        package.insert_part(Part::from_zip_entry(
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
        )?)?;
    }

    hydrate_content_types(&mut package, resource_limits)?;
    hydrate_relationships(&mut package, resource_limits)?;
    core_presentation::hydrate_package_slide_ids(&mut package);
    Ok(package)
}

fn apply_dirty_parts(package: &mut Package, dirty_parts: &BTreeSet<PartName>) {
    for part_name in dirty_parts {
        package.mark_dirty(part_name.clone());
    }
}

fn document_id_from_entries(entries: &[RawEntry]) -> Result<String> {
    let content_types_bytes = entries
        .iter()
        .find(|entry| !entry.meta.is_dir && entry.name.as_str() == "/[Content_Types].xml")
        .map(|entry| entry.bytes.as_slice())
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;

    let ordinary_parts = entries
        .iter()
        .filter(|entry| !entry.meta.is_dir && entry.name.as_str() != "/[Content_Types].xml")
        .map(|entry| (entry.name.clone(), entry.bytes.as_slice()))
        .collect::<Vec<_>>();

    Ok(provenance_document_id(&ordinary_parts, content_types_bytes))
}

fn document_id_from_package(package: &Package, source_entries: &[RawEntry]) -> Result<String> {
    let package = package_with_serialized_control_parts(package, Some(source_entries))?;
    let content_types_name = content_types_part()?;
    let content_types = package
        .parts()
        .get(&content_types_name)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;
    let ordinary_parts = package
        .parts()
        .iter()
        .filter(|part| part.name() != &content_types_name)
        .map(|part| (part.name().clone(), part.bytes()))
        .collect::<Vec<_>>();

    Ok(provenance_document_id(
        &ordinary_parts,
        content_types.bytes(),
    ))
}

fn is_slide_part_name(part_name: &str) -> bool {
    let Some(file_name) = part_name.strip_prefix("/ppt/slides/slide") else {
        return false;
    };
    let Some(index) = file_name.strip_suffix(".xml") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

fn next_revision_value(revision: revision::Revision) -> Result<u32> {
    let next = revision.value().checked_add(1).ok_or_else(|| {
        Error::new(
            ErrorCode::InternalError,
            "Revision value exceeds the patch report schema range.",
        )
    })?;
    u32::try_from(next).map_err(|source| {
        Error::with_source(
            ErrorCode::InternalError,
            "Revision value exceeds the patch report schema range.",
            source,
        )
    })
}

fn package_to_zip_bytes(package: &Package) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let options = zip_writer::WriteOptions::default();
    let mut writer = PackageZipWriter::new(&mut output, &options);
    for (index, part) in package.parts().iter().enumerate() {
        let meta = dirty_zip_metadata(index, part.original_zip_entry_name(), part.bytes());
        writer.write_dirty(part.original_zip_entry_name(), part.bytes(), &meta)?;
    }
    writer.finish()?;
    Ok(output.into_inner())
}

fn write_package_to_writer<W>(
    source_bytes: &[u8],
    source_entries: &[RawEntry],
    package: &Package,
    output: W,
    options: &zip_writer::WriteOptions,
) -> Result<W>
where
    W: Write + std::io::Seek,
{
    let package = package_with_serialized_control_parts(package, Some(source_entries))?;
    let dirty_metas = dirty_metadata_for_package(&package, source_entries);
    let entries = write_entries_for_package(&package, source_entries, &dirty_metas);
    if source_entries.is_empty() {
        let mut writer = PackageZipWriter::new(output, options);
        for entry in entries {
            let WriteEntry::Dirty(entry) = entry else {
                continue;
            };
            writer.write_dirty(entry.name, entry.bytes, entry.meta)?;
        }
        return writer.finish();
    }

    zip_writer::write_writer_with_options(source_bytes, entries.as_slice(), output, options)
}

fn package_with_serialized_control_parts(
    package: &Package,
    source_entries: Option<&[RawEntry]>,
) -> Result<Package> {
    let mut package = package.clone();
    let serialize_all = source_entries.is_none_or(<[RawEntry]>::is_empty);
    serialize_content_types(&mut package, serialize_all)?;
    serialize_relationships(&mut package, serialize_all)?;
    Ok(package)
}

fn serialize_content_types(package: &mut Package, serialize_all: bool) -> Result<()> {
    let part_name = content_types_part()?;
    if !serialize_all && !package.dirty_parts().contains(&part_name) {
        return Ok(());
    }

    let bytes = package.content_types().to_xml()?;
    upsert_part_bytes(package, part_name, bytes)
}

fn serialize_relationships(package: &mut Package, serialize_all: bool) -> Result<()> {
    for (source, relationships) in package.relationships().relationships_by_source() {
        let part_name = source.relationship_part_name();
        if !serialize_all && !package.dirty_parts().contains(&part_name) {
            continue;
        }
        upsert_part_bytes(package, part_name, relationships_to_xml(&relationships)?)?;
    }

    Ok(())
}

fn upsert_part_bytes(package: &mut Package, part_name: PartName, bytes: Vec<u8>) -> Result<()> {
    if let Some(part) = package.parts_mut().get_mut(&part_name) {
        *part.bytes_mut() = bytes;
    } else {
        package.insert_zip_entry(part_name.zip_entry_name(), bytes)?;
    }
    Ok(())
}

fn dirty_metadata_for_package(
    package: &Package,
    source_entries: &[RawEntry],
) -> BTreeMap<PartName, ZipEntryMetadata> {
    let source_by_name = source_entries
        .iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    package
        .parts()
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let source = source_by_name.get(part.name());
            if !source_entries.is_empty()
                && source.is_some()
                && !package.dirty_parts().contains(part.name())
            {
                return None;
            }
            let meta = source.map_or_else(
                || dirty_zip_metadata(index, part.original_zip_entry_name(), part.bytes()),
                |entry| {
                    dirty_zip_metadata(
                        entry.meta.entry_index,
                        &entry.meta.original_name,
                        part.bytes(),
                    )
                },
            );
            Some((part.name().clone(), meta))
        })
        .collect()
}

fn write_entries_for_package<'a>(
    package: &'a Package,
    source_entries: &'a [RawEntry],
    dirty_metas: &'a BTreeMap<PartName, ZipEntryMetadata>,
) -> Vec<WriteEntry<'a>> {
    let parts_by_name = package
        .parts()
        .iter()
        .map(|part| (part.name().clone(), part))
        .collect::<BTreeMap<_, _>>();
    let mut write_entries = source_entries
        .iter()
        .filter_map(|entry| {
            if entry.meta.is_dir {
                return Some(WriteEntry::Clean(entry));
            }
            let part = parts_by_name.get(&entry.name)?;
            dirty_metas.get(part.name()).map_or_else(
                || Some(WriteEntry::Clean(entry)),
                |meta| {
                    Some(WriteEntry::Dirty(DirtyEntry {
                        name: entry.meta.original_name.as_str(),
                        bytes: part.bytes(),
                        meta,
                    }))
                },
            )
        })
        .collect::<Vec<_>>();

    for part in package.parts().iter() {
        if source_entries
            .iter()
            .any(|entry| entry.name == *part.name())
        {
            continue;
        }
        let Some(meta) = dirty_metas.get(part.name()) else {
            continue;
        };
        write_entries.push(WriteEntry::Dirty(DirtyEntry {
            name: part.original_zip_entry_name(),
            bytes: part.bytes(),
            meta,
        }));
    }

    write_entries
}

fn dirty_zip_metadata(index: usize, name: &str, bytes: &[u8]) -> ZipEntryMetadata {
    ZipEntryMetadata {
        entry_index: index,
        original_name: name.to_owned(),
        compression_method: zip::CompressionMethod::Deflated,
        crc32: 0,
        compressed_size: 0,
        uncompressed_size: bytes.len() as u64,
        last_modified: None,
        external_attrs: None,
        is_dir: false,
    }
}

fn hydrate_content_types(package: &mut Package, resource_limits: &ResourceLimits) -> Result<()> {
    let name = PartName::from_zip_entry("[Content_Types].xml")?;
    let part = package
        .parts()
        .get(&name)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;
    *package.content_types_mut() = ContentTypes::parse_with_limits(part.bytes(), resource_limits)?;
    Ok(())
}

fn hydrate_relationships(package: &mut Package, resource_limits: &ResourceLimits) -> Result<()> {
    let rels_parts = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part, bytes) in rels_parts {
        if rels_part.as_str() == "/_rels/.rels" {
            for relationship in parse_root_relationships(&bytes, resource_limits)? {
                package.push_relationship(relationship);
            }
        } else {
            let source = relationship_source_for(&rels_part)?;
            let set = RelationshipSet::parse_with_limits(&source, &bytes, resource_limits)?;
            package.relationships_mut().insert_set(set);
        }
    }

    Ok(())
}

fn parse_root_relationships(
    bytes: &[u8],
    resource_limits: &ResourceLimits,
) -> Result<Vec<Relationship>> {
    let document = parse_document_with_limits(bytes, resource_limits).map_err(|source| {
        let code = if source.code() == ErrorCode::ResourceLimitExceeded {
            source.code()
        } else {
            ErrorCode::UnsupportedPackage
        };
        Error::with_source(code, "Could not parse package root relationships.", source)
    })?;
    let root = document.root_element().ok_or_else(|| {
        Error::unsupported_package("Package root relationships part has no root element.")
    })?;
    if root.name.local_name != "Relationships" {
        return Err(Error::unsupported_package(
            "Package root relationships part root element is not Relationships.",
        ));
    }

    root.children
        .iter()
        .filter_map(|node| node.as_element())
        .filter(|element| element.name.local_name == "Relationship")
        .map(parse_root_relationship)
        .collect()
}

fn parse_root_relationship(element: &XmlElement) -> Result<Relationship> {
    let id = required_attr(element, "Id")?;
    let rel_type = required_attr(element, "Type")?;
    let target = required_attr(element, "Target")?;
    let target_mode = match optional_attr(element, "TargetMode") {
        None | Some("Internal") => TargetMode::Internal,
        Some("External") => TargetMode::External,
        Some(other) => {
            return Err(Error::unsupported_package(format!(
                "Package root relationship {id} has unsupported TargetMode {other}."
            )));
        }
    };

    Ok(match target_mode {
        TargetMode::Internal => {
            Relationship::internal(RelationshipSource::Package, id, rel_type, target)
        }
        TargetMode::External => {
            Relationship::external(RelationshipSource::Package, id, rel_type, target)
        }
    })
}

fn content_types_part() -> Result<PartName> {
    PartName::from_zip_entry("[Content_Types].xml")
}

fn relationship_source_for(rels_part: &PartName) -> Result<PartName> {
    let rels_path = rels_part.as_str();
    let Some((directory, file_name)) = rels_path.rsplit_once("/_rels/") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part} is not in an _rels directory."
        )));
    };
    let Some(source_file_name) = file_name.strip_suffix(".rels") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part} does not end with .rels."
        )));
    };

    PartName::from_zip_entry(format!("{directory}/{source_file_name}").as_str())
}

fn required_attr<'a>(element: &'a XmlElement, name: &str) -> Result<&'a str> {
    optional_attr(element, name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Relationship element is missing required attribute {name}."
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

struct RealOperationExecutor<'a> {
    media_inputs: &'a MediaInputs,
}

impl OperationExecutor for RealOperationExecutor<'_> {
    fn validate(&mut self, package: &Package, operation: &Operation) -> Result<PatchEffects> {
        let model = core_presentation::PresentationDocument::open(package.clone())?;
        validate_operation(package, &model, operation, self.media_inputs)?;
        Ok(PatchEffects::default())
    }

    fn apply(&mut self, package: &mut Package, operation: &Operation) -> Result<PatchEffects> {
        let model = core_presentation::PresentationDocument::open(package.clone())?;
        match operation {
            Operation::ReplaceText(operation) => {
                let target = resolve_element(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                ReplaceText::from(operation).apply(package, &target)
            }
            Operation::ReplaceNotesText(operation) => {
                let target = resolve_notes_slide(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                ReplaceNotesText::from(operation).apply(package, &target)
            }
            Operation::ReplaceTableCellText(operation) => {
                let target = resolve_table_cell(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                    operation.cell.row,
                    operation.cell.col,
                )?;
                ReplaceTableCellText::from(operation).apply(package, &target)
            }
            Operation::AddTextBox(operation) => {
                let target = resolve_slide(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                AddTextBox::from(operation).apply(package, &target)
            }
            Operation::MoveResizeElement(operation) => {
                let target = resolve_element(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                MoveResize::from(operation).apply(package, &target)
            }
            Operation::SetAltText(operation) => {
                let target = resolve_element(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                SetAltText::from(operation).apply(package, &target)
            }
            Operation::SetDocumentMetadata(operation) => {
                let target = resolve_core_properties(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector(),
                )?;
                SetDocumentMetadata::from(operation).apply(package, &target)
            }
            Operation::AddImage(operation) => {
                let target = resolve_slide(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                AddImage::from(operation).apply(package, &target, self.media_inputs)
            }
            Operation::ReplaceImage(operation) => {
                let target = resolve_element(
                    &model,
                    operation.operation_id.as_str(),
                    &operation.target_selector()?,
                )?;
                ReplaceImage::from(operation).apply(package, &target, self.media_inputs)
            }
        }
    }
}

fn append_media_input_warnings(report: &mut PatchReport, media_report: MediaInputReport) {
    report
        .warnings
        .extend(media_report.warnings.into_iter().map(|warning| {
            serde_json::json!({
                "category": "media_input",
                "code": "unused_media_ref",
                "media_ref": warning.media_ref,
                "message": warning.message,
            })
        }));
}

fn operation_media_ref(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::AddImage(operation) => Some(operation.media_ref.as_str()),
        Operation::ReplaceImage(operation) => Some(operation.media_ref.as_str()),
        Operation::ReplaceText(_)
        | Operation::ReplaceNotesText(_)
        | Operation::ReplaceTableCellText(_)
        | Operation::AddTextBox(_)
        | Operation::MoveResizeElement(_)
        | Operation::SetAltText(_)
        | Operation::SetDocumentMetadata(_) => None,
    }
}

fn normalize_agent_view_capabilities(value: &mut serde_json::Value) {
    let Some(capabilities) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("capabilities"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    capabilities.insert(
        "operations".to_owned(),
        serde_json::Value::Array(
            ALL_OP_NAMES
                .iter()
                .map(|op| serde_json::Value::String((*op).to_owned()))
                .collect(),
        ),
    );
}

fn validate_operation(
    package: &Package,
    model: &core_presentation::PresentationDocument,
    operation: &Operation,
    media_inputs: &MediaInputs,
) -> Result<()> {
    match operation {
        Operation::ReplaceText(operation) => {
            let target = resolve_element(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            ReplaceText::from(operation).validate(package, &target)
        }
        Operation::ReplaceNotesText(operation) => {
            let target = resolve_notes_slide(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            ReplaceNotesText::from(operation).validate(package, &target)
        }
        Operation::ReplaceTableCellText(operation) => {
            let target = resolve_table_cell(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
                operation.cell.row,
                operation.cell.col,
            )?;
            ReplaceTableCellText::from(operation).validate(package, &target)
        }
        Operation::AddTextBox(operation) => {
            let _target = resolve_slide(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            AddTextBox::from(operation).validate()
        }
        Operation::MoveResizeElement(operation) => {
            let _target = resolve_element(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            MoveResize::from(operation).validate()
        }
        Operation::SetAltText(operation) => {
            let target = resolve_element(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            SetAltText::from(operation).validate(package, &target)
        }
        Operation::SetDocumentMetadata(operation) => {
            let target = resolve_core_properties(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector(),
            )?;
            SetDocumentMetadata::from(operation).validate(package, &target)
        }
        Operation::AddImage(operation) => {
            let _target = resolve_slide(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            AddImage::from(operation).validate(media_inputs)
        }
        Operation::ReplaceImage(operation) => {
            let target = resolve_element(
                model,
                operation.operation_id.as_str(),
                &operation.target_selector()?,
            )?;
            ReplaceImage::from(operation).validate(package, &target, media_inputs)
        }
    }
}

fn resolve_element(
    model: &core_presentation::PresentationDocument,
    operation_id: &str,
    selector: &Selector,
) -> Result<pptx_compose_edit::operations::ResolvedElement> {
    match selectors::resolve(model, selector)
        .map_err(|error| with_operation_location(error, operation_id))?
    {
        ResolvedTarget::Element(target) => Ok(target),
        ResolvedTarget::Slide(_)
        | ResolvedTarget::NotesSlide(_)
        | ResolvedTarget::TableCell(_)
        | ResolvedTarget::MediaPart(_)
        | ResolvedTarget::CoreProperties(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to an element.",
        )),
    }
}

fn resolve_table_cell(
    model: &core_presentation::PresentationDocument,
    operation_id: &str,
    selector: &Selector,
    row: u32,
    col: u32,
) -> Result<pptx_compose_edit::operations::ResolvedTableCell> {
    let element = resolve_element(model, operation_id, selector)?;
    Ok(pptx_compose_edit::operations::ResolvedTableCell { element, row, col })
}

fn resolve_notes_slide(
    model: &core_presentation::PresentationDocument,
    operation_id: &str,
    selector: &Selector,
) -> Result<pptx_compose_edit::operations::ResolvedNotesSlide> {
    match selectors::resolve_notes_slide(model, selector)
        .map_err(|error| with_operation_location(error, operation_id))?
    {
        ResolvedTarget::NotesSlide(target) => Ok(target),
        ResolvedTarget::Element(_)
        | ResolvedTarget::TableCell(_)
        | ResolvedTarget::Slide(_)
        | ResolvedTarget::MediaPart(_)
        | ResolvedTarget::CoreProperties(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to a notes slide.",
        )),
    }
}

fn resolve_slide(
    model: &core_presentation::PresentationDocument,
    operation_id: &str,
    selector: &Selector,
) -> Result<pptx_compose_edit::operations::ResolvedSlide> {
    match selectors::resolve(model, selector)
        .map_err(|error| with_operation_location(error, operation_id))?
    {
        ResolvedTarget::Slide(target) => Ok(target),
        ResolvedTarget::Element(_)
        | ResolvedTarget::NotesSlide(_)
        | ResolvedTarget::TableCell(_)
        | ResolvedTarget::MediaPart(_)
        | ResolvedTarget::CoreProperties(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to a slide.",
        )),
    }
}

fn resolve_core_properties(
    model: &core_presentation::PresentationDocument,
    operation_id: &str,
    selector: &Selector,
) -> Result<pptx_compose_edit::operations::ResolvedCoreProperties> {
    match selectors::resolve(model, selector)
        .map_err(|error| with_operation_location(error, operation_id))?
    {
        ResolvedTarget::CoreProperties(target) => Ok(target),
        ResolvedTarget::Element(_)
        | ResolvedTarget::NotesSlide(_)
        | ResolvedTarget::TableCell(_)
        | ResolvedTarget::Slide(_)
        | ResolvedTarget::MediaPart(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to core properties.",
        )),
    }
}

fn with_operation_location(error: Error, operation_id: &str) -> Error {
    if operation_id.is_empty() {
        return error;
    }
    let mut location = error.details().location.clone();
    location
        .operation_id
        .get_or_insert_with(|| operation_id.to_owned());
    error.with_location(location)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use pptx_compose_core::{
        error::ErrorCode,
        opc::part_name::PartName,
        zip::{
            limits::{OpenOptions as CoreOpenOptions, ResourceLimits},
            reader::from_bytes_with_options,
        },
    };
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::{
        PresentationDocument, WriteOptions,
        capabilities::{CapabilitiesOptions, capabilities},
        package_from_entries_with_limits,
    };
    use crate::edit::patch::ALL_OP_NAMES;

    #[test]
    fn facade_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose");
    }

    #[test]
    fn agent_view_and_capabilities_advertise_canonical_operations() {
        let bytes = zip_entries([
            ("[Content_Types].xml", CONTENT_TYPES_WITH_SLIDE.as_bytes()),
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("ppt/presentation.xml", PRESENTATION_WITH_SLIDE.as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                PRESENTATION_RELS.as_bytes(),
            ),
            ("ppt/slides/slide1.xml", SLIDE.as_bytes()),
        ]);
        let document = PresentationDocument::from_bytes(&bytes).expect("minimal deck opens");
        let view = document.to_agent_json().expect("agent view builds");
        let view_operations = view["capabilities"]["operations"]
            .as_array()
            .expect("agent view operations are an array")
            .iter()
            .map(|op| op.as_str().expect("operation name is a string"))
            .collect::<Vec<_>>();
        let capability_document = capabilities(CapabilitiesOptions::new("pptx-compose", "0.1.0"));
        let capability_operations = capability_document
            .supported_operations
            .iter()
            .map(|op| op.op.as_str())
            .collect::<Vec<_>>();

        assert_eq!(view_operations, ALL_OP_NAMES);
        assert_eq!(capability_operations, ALL_OP_NAMES);
    }

    #[test]
    fn loaded_package_office_document_part_resolves_root_relationship_target() {
        let bytes = zip_entries([
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("ppt/presentation.xml", PRESENTATION.as_bytes()),
        ]);
        let limits = ResourceLimits::default();
        let entries = from_bytes_with_options(
            &bytes,
            &CoreOpenOptions {
                resource_limits: limits.clone(),
            },
        )
        .expect("minimal deck zip opens");
        let package =
            package_from_entries_with_limits(&entries, &limits).expect("minimal deck hydrates");

        assert_eq!(
            package
                .office_document_part()
                .expect("office document resolves")
                .as_str(),
            "/ppt/presentation.xml"
        );
    }

    #[test]
    fn validation_on_write_blocks_dirty_malformed_xml_without_writing_bytes() {
        let bytes = zip_entries([
            ("[Content_Types].xml", CONTENT_TYPES_WITH_SLIDE.as_bytes()),
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("ppt/presentation.xml", PRESENTATION_WITH_SLIDE.as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                PRESENTATION_RELS.as_bytes(),
            ),
            ("ppt/slides/slide1.xml", b"<p:sld>"),
        ]);
        let mut document =
            PresentationDocument::from_bytes(&bytes).expect("malformed dirty slide deck opens");
        document.dirty_parts.insert(
            PartName::from_zip_entry("ppt/slides/slide1.xml").expect("slide part name is valid"),
        );

        let result = document.write_vec_with_options(WriteOptions {
            validate: true,
            ..WriteOptions::default()
        });

        let error = result.expect_err("blocking validation findings fail the write");
        assert_eq!(error.code(), ErrorCode::ValidationFailed);
    }

    fn zip_entries<const N: usize>(entries: [(&str, &[u8]); N]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, data) in entries {
                writer.start_file(name, options).expect("start ZIP entry");
                writer.write_all(data).expect("write ZIP entry");
            }
            writer.finish().expect("finish ZIP");
        }
        bytes
    }

    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
</Types>"#;

    const CONTENT_TYPES_WITH_SLIDE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#;

    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#;

    const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#;

    const PRESENTATION_WITH_SLIDE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#;

    const PRESENTATION_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

    const SLIDE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr>
        <p:cNvPr id="1" name=""/>
        <p:cNvGrpSpPr/>
        <p:nvPr/>
      </p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
</p:sld>"#;
}
