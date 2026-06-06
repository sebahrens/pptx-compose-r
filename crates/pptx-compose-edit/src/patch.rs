use std::{
    collections::{BTreeMap, HashSet},
    error, fmt,
};

use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    validation::{ValidationMode, validate_package},
};
use pptx_compose_json::{
    schema_versions::{PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION},
    schemas::{
        OperationReport, OperationStatus, OperationTarget, PatchReport, PatchStatus,
        ValidationReport,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::reports::{has_blocking_findings, patch_validation_summary, validation_report};

pub const PATCH_SCHEMA: &str = "pptx-compose.patch.v1";
pub const PATCH_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub schema: String,
    pub version: u32,
    pub document_id: String,
    pub base_revision: u32,
    pub client_request_id: String,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    ReplaceText(ReplaceTextOperation),
    AddTextBox(AddTextBoxOperation),
    MoveResizeElement(MoveResizeElementOperation),
    SetAltText(SetAltTextOperation),
    AddImage(AddImageOperation),
    ReplaceImage(ReplaceImageOperation),
}

impl Operation {
    #[must_use]
    pub fn operation_id(&self) -> &str {
        match self {
            Self::ReplaceText(operation) => &operation.operation_id,
            Self::AddTextBox(operation) => &operation.operation_id,
            Self::MoveResizeElement(operation) => &operation.operation_id,
            Self::SetAltText(operation) => &operation.operation_id,
            Self::AddImage(operation) => &operation.operation_id,
            Self::ReplaceImage(operation) => &operation.operation_id,
        }
    }

    #[must_use]
    pub const fn op_name(&self) -> &'static str {
        match self {
            Self::ReplaceText(_) => "replace_text",
            Self::AddTextBox(_) => "add_text_box",
            Self::MoveResizeElement(_) => "move_resize_element",
            Self::SetAltText(_) => "set_alt_text",
            Self::AddImage(_) => "add_image",
            Self::ReplaceImage(_) => "replace_image",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceTextOperation {
    pub operation_id: String,
    pub element_id: String,
    pub text: String,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub current_text_match: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ReplaceTextMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_policy: Option<FormatPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_policy: Option<OverflowPolicy>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceTextMode {
    WholeElement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormatPolicy {
    PreserveExistingRuns,
    PreserveFirstRun,
    SingleRunDefaultStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    Allow,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddTextBoxOperation {
    pub operation_id: String,
    pub slide_id: String,
    pub text: String,
    pub bounds: Bounds,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<TextBoxStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert: Option<InsertOptions>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveResizeElementOperation {
    pub operation_id: String,
    pub element_id: String,
    pub bounds: Bounds,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAltTextOperation {
    pub operation_id: String,
    pub element_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddImageOperation {
    pub operation_id: String,
    pub slide_id: String,
    pub media_ref: String,
    pub content_type: String,
    pub bounds: Bounds,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<ImageFit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe: Option<ImageDedupe>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceImageOperation {
    pub operation_id: String,
    pub element_id: String,
    pub media_ref: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_shared_mutation: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct TextBoxStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size_pt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_order: Option<ZOrder>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZOrder {
    Front,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    Stretch,
    Contain,
    Cover,
    OriginalSize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageDedupe {
    Never,
    Checksum,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentState {
    pub document_id: String,
    pub revision: u32,
}

impl DocumentState {
    #[must_use]
    pub fn new(document_id: impl Into<String>, revision: u32) -> Self {
        Self {
            document_id: document_id.into(),
            revision,
        }
    }
}

pub fn parse_patch(value: serde_json::Value) -> Result<Patch> {
    serde_json::from_value(value).map_err(|source| {
        Error::with_source(
            ErrorCode::InvalidInput,
            "Patch envelope is invalid.",
            source,
        )
    })
}

pub fn validate_envelope(patch: &Patch, doc: &DocumentState) -> Result<()> {
    if patch.schema != PATCH_SCHEMA {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            format!("Patch schema must be {PATCH_SCHEMA}."),
        ));
    }

    if patch.version != PATCH_VERSION {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            format!("Patch version must be {PATCH_VERSION}."),
        ));
    }

    if patch.document_id != doc.document_id || patch.base_revision != doc.revision {
        return Err(Error::stale_revision(
            "Patch document_id or base_revision does not match the current document.",
        ));
    }

    let mut operation_ids = HashSet::new();
    for operation in &patch.operations {
        let operation_id = operation.operation_id();
        if operation_id.is_empty() {
            return Err(invalid_operation_id(
                "Patch operations must include a non-empty operation_id.",
                operation_id,
            ));
        }
        if !operation_ids.insert(operation_id) {
            return Err(invalid_operation_id(
                format!("Patch operation_id {operation_id} is duplicated."),
                operation_id,
            ));
        }
    }

    Ok(())
}

fn invalid_operation_id(message: impl Into<String>, operation_id: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message).with_location(ErrorLocation {
        operation_id: Some(operation_id.to_owned()),
        ..ErrorLocation::default()
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchContext {
    pub document_id: String,
    pub base_revision: u32,
    pub new_document_id: String,
    pub new_revision: u32,
    pub dry_run: bool,
}

impl PatchContext {
    #[must_use]
    pub fn new(
        document_id: impl Into<String>,
        base_revision: u32,
        new_document_id: impl Into<String>,
        new_revision: u32,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            base_revision,
            new_document_id: new_document_id.into(),
            new_revision,
            dry_run: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatchEffects {
    pub changed_parts: Vec<String>,
    pub target: Option<OperationTarget>,
    pub created_element_ids: Vec<String>,
    pub warnings: Vec<serde_json::Value>,
}

pub trait OperationExecutor {
    fn validate(&mut self, package: &Package, operation: &Operation) -> Result<PatchEffects>;

    fn apply(&mut self, package: &mut Package, operation: &Operation) -> Result<PatchEffects>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyPatchResult {
    pub package: WritablePackage,
    pub report: PatchReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WritablePackage {
    package: Package,
}

impl WritablePackage {
    #[must_use]
    pub const fn package(&self) -> &Package {
        &self.package
    }

    #[must_use]
    pub fn into_inner(self) -> Package {
        self.package
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidationFailedReport {
    pub report: ValidationReport,
}

impl fmt::Display for ValidationFailedReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "validation failed with {} error(s) and {} fatal finding(s)",
            self.report.summary.errors, self.report.summary.fatal
        )
    }
}

impl error::Error for ValidationFailedReport {}

pub fn apply_package_patch<F>(
    package: Package,
    context: PatchContext,
    apply: F,
) -> Result<ApplyPatchResult>
where
    F: FnOnce(&mut Package) -> Result<PatchEffects>,
{
    let mut edited = package;
    let effects = apply(&mut edited)?;
    let validation = validate_for_write(&edited, &context.new_document_id, context.new_revision)?;

    if has_blocking_findings(&validation) {
        return Err(validation_failed(validation));
    }

    Ok(ApplyPatchResult {
        package: WritablePackage { package: edited },
        report: PatchReport {
            schema: PATCH_REPORT_SCHEMA.to_owned(),
            version: PATCH_REPORT_VERSION,
            status: if context.dry_run {
                PatchStatus::DryRunSuccess
            } else {
                PatchStatus::Applied
            },
            dry_run: context.dry_run,
            document_id: context.document_id,
            base_revision: context.base_revision,
            new_document_id: context.new_document_id,
            new_revision: context.new_revision,
            operation_reports: Vec::new(),
            changed_parts: effects.changed_parts,
            validation: patch_validation_summary(&validation),
        },
    })
}

pub fn apply_patch<E>(
    package: &mut Package,
    mut context: PatchContext,
    patch: &Patch,
    dry_run: bool,
    executor: &mut E,
) -> Result<PatchReport>
where
    E: OperationExecutor,
{
    context.dry_run = dry_run;
    let mut staged = package.clone();
    let mut operation_reports = Vec::with_capacity(patch.operations.len());
    let mut changed_parts = Vec::new();

    for operation in &patch.operations {
        let validation_effects = executor.validate(&staged, operation)?;
        let effects = if dry_run {
            validation_effects
        } else {
            executor.apply(&mut staged, operation)?
        };

        changed_parts.extend(effects.changed_parts.iter().cloned());
        operation_reports.push(operation_report(
            operation,
            if dry_run {
                OperationStatus::Validated
            } else {
                OperationStatus::Applied
            },
            effects,
        ));
    }

    changed_parts.sort();
    changed_parts.dedup();

    let wrote_part = !changed_parts.is_empty();
    let report_revision = if dry_run || !wrote_part {
        context.base_revision
    } else {
        context.new_revision
    };
    let validation = validate_for_write(&staged, &context.new_document_id, report_revision)?;

    if has_blocking_findings(&validation) {
        return Err(validation_failed(validation));
    }

    if !dry_run {
        *package = staged;
    }

    Ok(PatchReport {
        schema: PATCH_REPORT_SCHEMA.to_owned(),
        version: PATCH_REPORT_VERSION,
        status: if dry_run {
            PatchStatus::DryRunSuccess
        } else {
            PatchStatus::Applied
        },
        dry_run,
        document_id: context.document_id,
        base_revision: context.base_revision,
        new_document_id: context.new_document_id,
        new_revision: report_revision,
        operation_reports,
        changed_parts,
        validation: patch_validation_summary(&validation),
    })
}

fn operation_report(
    operation: &Operation,
    status: OperationStatus,
    effects: PatchEffects,
) -> OperationReport {
    OperationReport {
        operation_id: operation.operation_id().to_owned(),
        op: operation.op_name().to_owned(),
        status,
        target: effects.target.unwrap_or_else(unknown_target),
        changed_parts: effects.changed_parts,
        created_element_ids: effects.created_element_ids,
        warnings: effects.warnings,
    }
}

fn unknown_target() -> OperationTarget {
    OperationTarget {
        slide_id: String::new(),
        element_id: String::new(),
        part: String::new(),
    }
}

pub fn validate_for_write(
    package: &Package,
    document_id: impl Into<String>,
    revision: u32,
) -> Result<ValidationReport> {
    validation_report(
        validate_package(package, ValidationMode::Edited),
        document_id,
        revision,
    )
}

fn validation_failed(report: ValidationReport) -> Error {
    Error::with_source(
        ErrorCode::ValidationFailed,
        "Edited package failed validation and was not made writable.",
        ValidationFailedReport { report },
    )
    .with_suggestion("Inspect the validation report, fix the invalid edit, and retry the patch.")
}

#[cfg(test)]
#[test]
fn blocks_write_on_invalid() {
    test_support::blocks_write_on_invalid();
}

#[cfg(test)]
#[test]
fn envelope_and_stale() {
    let patch = parse_patch(serde_json::json!({
        "schema": PATCH_SCHEMA,
        "version": PATCH_VERSION,
        "document_id": "sha256:current",
        "base_revision": 3,
        "client_request_id": "agent-run-001",
        "operations": [
            {
                "operation_id": "op-1",
                "op": "replace_text",
                "element_id": "slide-1:shape-4",
                "text": "Updated title"
            }
        ]
    }))
    .expect("well-formed patch envelope parses");

    let current = DocumentState::new("sha256:current", 3);
    validate_envelope(&patch, &current).expect("current revision patch validates");

    let stale = DocumentState::new("sha256:current", 4);
    let error = validate_envelope(&patch, &stale).expect_err("base_revision mismatch is stale");
    assert_eq!(error.code(), ErrorCode::StalePatch);

    let error = parse_patch(serde_json::json!({
        "schema": PATCH_SCHEMA,
        "version": PATCH_VERSION,
        "document_id": "sha256:current",
        "base_revision": 3,
        "client_request_id": "agent-run-001",
        "unknown": true,
        "operations": []
    }))
    .expect_err("unknown top-level patch field is rejected");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[cfg(test)]
#[test]
fn apply_is_atomic() {
    test_support::apply_is_atomic();
}

#[cfg(test)]
mod test_support {
    use pptx_compose_core::{
        error::{Error, ErrorCode, Result},
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
    };
    use pptx_compose_json::schemas::{
        FindingCode, OperationStatus, OperationTarget, ValidationStatus,
    };

    use super::{
        AddTextBoxOperation, Operation, OperationExecutor, PATCH_SCHEMA, PATCH_VERSION, Patch,
        PatchContext, PatchEffects, apply_package_patch,
    };

    const IMAGE_REL_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

    pub fn blocks_write_on_invalid() {
        let package = base_package();
        let source = part("ppt/slides/slide1.xml");
        let error = apply_package_patch(package.clone(), context(), |package| {
            package.push_relationship(Relationship::internal(
                RelationshipSource::Part(source),
                "rId1",
                IMAGE_REL_TYPE,
                "../media/missing.png",
            ));
            Ok(PatchEffects {
                changed_parts: vec!["ppt/slides/_rels/slide1.xml.rels".to_owned()],
                ..PatchEffects::default()
            })
        })
        .expect_err("dangling relationship blocks writable package");

        assert_eq!(error.code(), ErrorCode::ValidationFailed);

        let clean = apply_package_patch(package, context(), |_| Ok(PatchEffects::default()))
            .expect("valid patch returns writable package");

        assert_eq!(clean.report.validation.status, ValidationStatus::Valid);
        assert_eq!(clean.report.validation.errors, 0);
        assert!(!clean.package.package().parts().is_empty());
    }

    #[test]
    fn warning_only_validation_passes_through_report() {
        let mut package = base_package();
        package.push_relationship(Relationship::external(
            RelationshipSource::Part(part("ppt/slides/slide1.xml")),
            "rId2",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
            "https://example.test/",
        ));

        let result = apply_package_patch(package, context(), |_| Ok(PatchEffects::default()))
            .expect("warning-only validation is writable");

        assert_eq!(result.report.validation.status, ValidationStatus::Valid);
        assert_eq!(result.report.validation.errors, 0);
        assert_eq!(result.report.validation.warnings, 1);
    }

    #[test]
    fn validate_for_write_reports_blocking_findings() {
        let mut package = base_package();
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(part("ppt/slides/slide1.xml")),
            "rId1",
            IMAGE_REL_TYPE,
            "../media/missing.png",
        ));

        let report = super::validate_for_write(&package, "sha256:test", 1)
            .expect("validation report builds");

        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(report.findings.iter().any(|finding| {
            finding.code == FindingCode::DanglingInternalRelationship && finding.blocking
        }));
    }

    pub fn apply_is_atomic() {
        let mut package = base_package();
        let original = package.clone();
        let patch = two_op_patch();
        let mut executor = FailingSecondOp;

        let error = super::apply_patch(&mut package, context(), &patch, false, &mut executor)
            .expect_err("second operation failure aborts patch");

        assert_eq!(error.code(), ErrorCode::UnsupportedEdit);
        assert_eq!(package, original);

        let mut executor = SuccessfulOps;
        let report = super::apply_patch(&mut package, context(), &patch, false, &mut executor)
            .expect("all successful operations apply atomically");

        assert_eq!(
            report.status,
            pptx_compose_json::schemas::PatchStatus::Applied
        );
        assert_eq!(report.operation_reports.len(), 2);
        assert!(
            report
                .operation_reports
                .iter()
                .all(|operation| operation.status == OperationStatus::Applied)
        );
        assert_ne!(package, original);
    }

    struct FailingSecondOp;

    impl OperationExecutor for FailingSecondOp {
        fn validate(&mut self, _package: &Package, operation: &Operation) -> Result<PatchEffects> {
            if operation.operation_id() == "op-2" {
                return Err(Error::new(
                    ErrorCode::UnsupportedEdit,
                    "Test operation op-2 is unsupported.",
                ));
            }

            Ok(effects(operation.operation_id()))
        }

        fn apply(&mut self, package: &mut Package, operation: &Operation) -> Result<PatchEffects> {
            package.mark_dirty(part("ppt/slides/slide1.xml"));
            Ok(effects(operation.operation_id()))
        }
    }

    struct SuccessfulOps;

    impl OperationExecutor for SuccessfulOps {
        fn validate(&mut self, _package: &Package, operation: &Operation) -> Result<PatchEffects> {
            Ok(effects(operation.operation_id()))
        }

        fn apply(&mut self, package: &mut Package, operation: &Operation) -> Result<PatchEffects> {
            package.mark_dirty(part("ppt/slides/slide1.xml"));
            Ok(effects(operation.operation_id()))
        }
    }

    fn effects(operation_id: &str) -> PatchEffects {
        PatchEffects {
            changed_parts: vec!["ppt/slides/slide1.xml".to_owned()],
            target: Some(OperationTarget {
                slide_id: "slide-1".to_owned(),
                element_id: format!("slide-1:{operation_id}"),
                part: "ppt/slides/slide1.xml".to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn two_op_patch() -> Patch {
        Patch {
            schema: PATCH_SCHEMA.to_owned(),
            version: PATCH_VERSION,
            document_id: "sha256:old".to_owned(),
            base_revision: 1,
            client_request_id: "agent-run-001".to_owned(),
            operations: vec![
                Operation::AddTextBox(AddTextBoxOperation {
                    operation_id: "op-1".to_owned(),
                    slide_id: "slide-1".to_owned(),
                    text: "One".to_owned(),
                    bounds: super::Bounds {
                        x: 0,
                        y: 0,
                        cx: 1,
                        cy: 1,
                    },
                    name: None,
                    alt_text: None,
                    style: None,
                    insert: None,
                }),
                Operation::AddTextBox(AddTextBoxOperation {
                    operation_id: "op-2".to_owned(),
                    slide_id: "slide-1".to_owned(),
                    text: "Two".to_owned(),
                    bounds: super::Bounds {
                        x: 0,
                        y: 0,
                        cx: 1,
                        cy: 1,
                    },
                    name: None,
                    alt_text: None,
                    style: None,
                    insert: None,
                }),
            ],
        }
    }

    fn base_package() -> Package {
        let mut package = Package::new();
        package
            .insert_zip_entry("ppt/slides/slide1.xml", b"<p:sld/>".to_vec())
            .expect("slide part inserted");
        package.content_types_mut().insert_override(
            part("ppt/slides/slide1.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        );
        package
    }

    fn context() -> PatchContext {
        PatchContext::new("sha256:old", 1, "sha256:new", 2)
    }

    fn part(name: &str) -> PartName {
        PartName::from_zip_entry(name).expect("valid fixture part name")
    }
}
