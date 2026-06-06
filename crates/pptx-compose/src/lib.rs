#![deny(warnings)]

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions as FsOpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

mod options;

pub use options::{AgentViewOptions, ApplyPatchOptions, OpenOptions, WriteOptions};
pub use pptx_compose_core as core;
pub use pptx_compose_core::zip::writer::WriteMode;
pub use pptx_compose_edit as edit;
pub use pptx_compose_edit::{media_inputs::MediaInputs, patch::Patch};
pub use pptx_compose_json as json;

use core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::{
        content_types::ContentTypes,
        package::Package,
        part::Part,
        part_name::PartName,
        relationships::{Relationship, RelationshipSet, RelationshipSource, TargetMode},
    },
    pptx::presentation as core_presentation,
    provenance::{document_id::document_id as provenance_document_id, revision},
    xml::{document::XmlElement, parser::parse_document},
    zip::{
        ZipEntryMetadata,
        reader::{RawEntry, from_bytes},
        sniff::sniff_package,
        writer::{self as zip_writer, DirtyEntry, PackageZipWriter, WriteEntry},
    },
};
use pptx_compose_edit::{
    operations::{
        ResolvedTarget, add_image::AddImage, add_text_box::AddTextBox, move_resize::MoveResize,
        replace_image::ReplaceImage, replace_text::ReplaceText, set_alt_text::SetAltText,
    },
    patch::{DocumentState, Operation, OperationExecutor, PatchEffects, validate_envelope},
    selectors::{self, Selector},
};
use pptx_compose_json::{
    agent_view::views::ViewRequest,
    schemas::{PatchReport, ValidationReport},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationDocument {
    source_path: Option<PathBuf>,
    source_bytes: Vec<u8>,
    entries: Vec<RawEntry>,
    dirty_parts: BTreeSet<PartName>,
}

impl PresentationDocument {
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_path_with_options(path, OpenOptions::default())
    }

    pub fn open_path_with_options(path: impl AsRef<Path>, _options: OpenOptions) -> Result<Self> {
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

    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let source_bytes = bytes.as_ref().to_vec();
        sniff_package(&mut Cursor::new(source_bytes.as_slice()))?;
        let entries = from_bytes(&source_bytes)?;
        Ok(Self {
            source_path: None,
            source_bytes,
            entries,
            dirty_parts: BTreeSet::new(),
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

    pub fn to_agent_json(&self) -> Result<serde_json::Value> {
        self.to_agent_json_with_options(AgentViewOptions::default())
    }

    pub fn to_agent_json_with_options(
        &self,
        options: AgentViewOptions,
    ) -> Result<serde_json::Value> {
        let package = package_from_entries(&self.entries)?;
        let model = core_presentation::PresentationDocument::open(package)?;
        pptx_compose_json::agent_view::views::build_view(
            &model,
            ViewRequest {
                mode: options.mode,
                slide_id: options.slide_id,
                element_id: options.element_id,
                cursor: options.cursor,
                limit: options.limit,
            },
        )
        .map_err(json_error)
    }

    pub fn to_legacy_json(&self) -> Result<serde_json::Value> {
        let package = package_from_entries(&self.entries)?;
        pptx_compose_json::legacy_path_map::to_legacy_map(&package).map_err(json_error)
    }

    pub fn from_legacy_json(value: serde_json::Value) -> Result<Self> {
        let package =
            pptx_compose_json::legacy_path_map::from_legacy_map(value).map_err(json_error)?;
        let bytes = package_to_zip_bytes(&package)?;
        Self::from_bytes(bytes)
    }

    pub fn write_vec_with_options(&self, options: WriteOptions) -> Result<Vec<u8>> {
        if options.validate {
            let _report = self.validate()?;
        }
        let entries = self.write_entries();
        let output = Cursor::new(Vec::new());
        let output = zip_writer::write_writer_with_options(
            &self.source_bytes,
            entries.as_slice(),
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
        if options.validate {
            let _report = self.validate()?;
        }

        let document_id = document_id_from_entries(&self.entries)?;
        let revision = u32::try_from(revision::on_open().value()).map_err(|source| {
            Error::with_source(
                ErrorCode::InternalError,
                "Revision value exceeds the patch report schema range.",
                source,
            )
        })?;
        validate_envelope(&patch, &DocumentState::new(document_id.clone(), revision))?;
        if options.dry_run {
            let validation = self.validate()?;
            return Ok(PatchReport {
                schema: pptx_compose_json::schema_versions::PATCH_REPORT_SCHEMA.to_owned(),
                version: pptx_compose_json::schema_versions::PATCH_REPORT_VERSION,
                status: pptx_compose_json::schemas::PatchStatus::DryRunSuccess,
                dry_run: true,
                document_id: document_id.clone(),
                base_revision: revision,
                new_document_id: document_id,
                new_revision: revision,
                operation_reports: Vec::new(),
                changed_parts: Vec::new(),
                validation: pptx_compose_edit::reports::patch_validation_summary(&validation),
            });
        }
        let package = package_from_entries(&self.entries)?;
        validate_patch_operations(&package, &patch.operations, &media)?;

        let mut package = package_from_entries(&self.entries)?;
        let mut executor = RealOperationExecutor {
            media_inputs: &media,
        };
        let report = pptx_compose_edit::patch::apply_patch(
            &mut package,
            pptx_compose_edit::patch::PatchContext::new(
                document_id.clone(),
                revision,
                document_id,
                revision,
            ),
            &patch,
            options.dry_run,
            &mut executor,
        )?;
        self.replace_entries_from_package(&package)?;
        Ok(report)
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
        let output = FsOpenOptions::new()
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
            let _report = self.validate()?;
        }
        let entries = self.write_entries();
        zip_writer::write_writer_with_options(
            &self.source_bytes,
            entries.as_slice(),
            output,
            &zip_writer::WriteOptions {
                mode: options.mode,
                ..zip_writer::WriteOptions::default()
            },
        )
    }

    pub fn validate(&self) -> Result<ValidationReport> {
        let package = package_from_entries(&self.entries)?;
        pptx_compose_edit::reports::validation_report(
            core::validation::validate_package(&package, core::validation::ValidationMode::Edited),
            document_id_from_entries(&self.entries)?,
            u32::try_from(revision::on_open().value()).map_err(|source| {
                Error::with_source(
                    ErrorCode::InternalError,
                    "Revision value exceeds the validation report schema range.",
                    source,
                )
            })?,
        )
    }

    pub fn write_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_path_with_options(path, WriteOptions::default())
    }

    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    fn write_entries(&self) -> Vec<WriteEntry<'_>> {
        self.entries
            .iter()
            .map(|entry| {
                if self.dirty_parts.contains(&entry.name) {
                    WriteEntry::Dirty(DirtyEntry {
                        name: entry.meta.original_name.as_str(),
                        bytes: entry.bytes.as_slice(),
                        meta: &entry.meta,
                    })
                } else {
                    WriteEntry::Clean(entry)
                }
            })
            .collect()
    }

    fn replace_entries_from_package(&mut self, package: &Package) -> Result<()> {
        for entry in &mut self.entries {
            if let Some(part) = package.parts().get(&entry.name) {
                entry.bytes = part.bytes().to_vec();
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

fn temp_sibling_path(output_path: &Path) -> PathBuf {
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output.pptx");
    let suffix = format!("{}.{}.tmp", std::process::id(), unique_counter());
    output_path.with_file_name(format!(".{file_name}.{suffix}"))
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
        pptx_compose_json::schemas::JsonError::NotFound { kind, id } => Error::new(
            ErrorCode::SelectorNotFound,
            format!("{kind} `{id}` was not found."),
        ),
    }
}

fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        package.insert_part(Part::from_zip_entry(
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
        )?)?;
    }

    hydrate_content_types(&mut package)?;
    hydrate_relationships(&mut package)?;
    Ok(package)
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

fn package_to_zip_bytes(package: &Package) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    {
        let options = zip_writer::WriteOptions::default();
        let mut writer = PackageZipWriter::new(&mut output, &options);
        for (index, part) in package.parts().iter().enumerate() {
            let meta = dirty_zip_metadata(index, part.original_zip_entry_name(), part.bytes());
            writer.write_dirty(part.original_zip_entry_name(), part.bytes(), &meta)?;
        }
        writer.finish()?;
    }
    Ok(output.into_inner())
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

fn hydrate_content_types(package: &mut Package) -> Result<()> {
    let name = PartName::from_zip_entry("[Content_Types].xml")?;
    let part = package
        .parts()
        .get(&name)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;
    *package.content_types_mut() = ContentTypes::parse(part.bytes())?;
    Ok(())
}

fn hydrate_relationships(package: &mut Package) -> Result<()> {
    let rels_parts = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part, bytes) in rels_parts {
        if rels_part.as_str() == "/_rels/.rels" {
            for relationship in parse_root_relationships(&bytes)? {
                package.push_relationship(relationship);
            }
        } else {
            let source = relationship_source_for(&rels_part)?;
            let set = RelationshipSet::parse(&source, &bytes)?;
            package.relationships_mut().insert_set(set);
        }
    }

    Ok(())
}

fn parse_root_relationships(bytes: &[u8]) -> Result<Vec<Relationship>> {
    let document = parse_document(bytes).map_err(|source| {
        Error::with_source(
            source.code(),
            "Could not parse package root relationships.",
            source,
        )
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
                let target = resolve_element(&model, &operation.element_id)?;
                ReplaceText::from(operation).apply(package, &target)
            }
            Operation::AddTextBox(operation) => {
                let target = resolve_slide(&model, &operation.slide_id)?;
                AddTextBox::from(operation).apply(package, &target)
            }
            Operation::MoveResizeElement(operation) => {
                let target = resolve_element(&model, &operation.element_id)?;
                MoveResize::from(operation).apply(package, &target)
            }
            Operation::SetAltText(operation) => {
                let target = resolve_element(&model, &operation.element_id)?;
                SetAltText::from(operation).apply(package, &target)
            }
            Operation::AddImage(operation) => {
                let target = resolve_slide(&model, &operation.slide_id)?;
                AddImage::from(operation).apply(package, &target, self.media_inputs)
            }
            Operation::ReplaceImage(operation) => {
                let target = resolve_element(&model, &operation.element_id)?;
                ReplaceImage::from(operation).apply(package, &target, self.media_inputs)
            }
        }
    }
}

fn validate_patch_operations(
    package: &Package,
    operations: &[Operation],
    media_inputs: &MediaInputs,
) -> Result<()> {
    media_inputs.check_references(
        operations.iter().filter_map(operation_media_ref),
        pptx_compose_edit::media_inputs::ExtraBindingPolicy::Warn,
    )?;

    let model = core_presentation::PresentationDocument::open(package.clone())?;
    for operation in operations {
        validate_operation(package, &model, operation, media_inputs)?;
    }
    Ok(())
}

fn operation_media_ref(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::AddImage(operation) => Some(operation.media_ref.as_str()),
        Operation::ReplaceImage(operation) => Some(operation.media_ref.as_str()),
        Operation::ReplaceText(_)
        | Operation::AddTextBox(_)
        | Operation::MoveResizeElement(_)
        | Operation::SetAltText(_) => None,
    }
}

fn validate_operation(
    package: &Package,
    model: &core_presentation::PresentationDocument,
    operation: &Operation,
    media_inputs: &MediaInputs,
) -> Result<()> {
    match operation {
        Operation::ReplaceText(operation) => {
            let target = resolve_element(model, &operation.element_id)?;
            ReplaceText::from(operation).validate(package, &target)
        }
        Operation::AddTextBox(operation) => {
            let _target = resolve_slide(model, &operation.slide_id)?;
            AddTextBox::from(operation).validate()
        }
        Operation::MoveResizeElement(operation) => {
            let _target = resolve_element(model, &operation.element_id)?;
            MoveResize::from(operation).validate()
        }
        Operation::SetAltText(operation) => {
            let target = resolve_element(model, &operation.element_id)?;
            SetAltText::from(operation).validate(package, &target)
        }
        Operation::AddImage(operation) => {
            let _target = resolve_slide(model, &operation.slide_id)?;
            let media = media_inputs
                .resolve(&operation.media_ref)
                .map_err(|error| {
                    error.with_location(ErrorLocation {
                        operation_id: Some(operation.operation_id.clone()),
                        operation: Some("add_image".to_owned()),
                        ..ErrorLocation::default()
                    })
                })?;
            if media.content_type != operation.content_type {
                return Err(Error::new(
                    ErrorCode::UnsupportedMediaType,
                    format!(
                        "add_image content_type `{}` does not match bound media_ref `{}` content type `{}`.",
                        operation.content_type, operation.media_ref, media.content_type
                    ),
                ));
            }
            Ok(())
        }
        Operation::ReplaceImage(operation) => {
            let target = resolve_element(model, &operation.element_id)?;
            ReplaceImage::from(operation).validate(package, &target, media_inputs)
        }
    }
}

fn resolve_element(
    model: &core_presentation::PresentationDocument,
    element_id: &str,
) -> Result<pptx_compose_edit::operations::ResolvedElement> {
    let selector = Selector::ElementId {
        id: element_id.to_owned(),
        guards: None,
    };
    match selectors::resolve(model, &selector)? {
        ResolvedTarget::Element(target) => Ok(target),
        ResolvedTarget::Slide(_) | ResolvedTarget::MediaPart(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to an element.",
        )),
    }
}

fn resolve_slide(
    model: &core_presentation::PresentationDocument,
    slide_id: &str,
) -> Result<pptx_compose_edit::operations::ResolvedSlide> {
    let selector = Selector::SlideId {
        id: slide_id.to_owned(),
        guards: None,
    };
    match selectors::resolve(model, &selector)? {
        ResolvedTarget::Slide(target) => Ok(target),
        ResolvedTarget::Element(_) | ResolvedTarget::MediaPart(_) => Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            "Selector did not resolve to a slide.",
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn facade_crate_compiles() {
        let crate_name = env!("CARGO_PKG_NAME");
        assert_eq!(crate_name, "pptx-compose");
    }
}
