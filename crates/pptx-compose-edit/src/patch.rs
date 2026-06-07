use std::{collections::HashSet, error, fmt};

use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    validation::{ValidationMode, validate_package},
};
use pptx_compose_json::{
    schema_versions::{PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION},
    schemas::{
        ErrorView, OperationReport, OperationStatus, OperationTarget, PatchReport, PatchStatus,
        ValidationReport,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    reports::{has_blocking_findings, patch_validation_summary, validation_report},
    selectors::{RunSelector, Selector},
};

pub const PATCH_SCHEMA: &str = "pptx-compose.patch.v1";
pub const PATCH_VERSION: u32 = 1;
pub const ALL_OP_NAMES: [&str; 7] = [
    "replace_text",
    "add_text_box",
    "move_resize_element",
    "set_alt_text",
    "set_document_metadata",
    "add_image",
    "replace_image",
];

pub fn patch_json_schema() -> Result<Value> {
    let schema = schemars::schema_for!(Patch);
    let mut value = serde_json::to_value(schema).map_err(|source| {
        Error::with_source(
            ErrorCode::InternalError,
            "Could not serialize patch JSON schema.",
            source,
        )
    })?;
    if let Some(object) = value.as_object_mut() {
        object.insert("$id".to_owned(), Value::String(PATCH_SCHEMA.to_owned()));
    }
    Ok(value)
}

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
    SetDocumentMetadata(SetDocumentMetadataOperation),
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
            Self::SetDocumentMetadata(operation) => &operation.operation_id,
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
            Self::SetDocumentMetadata(_) => "set_document_metadata",
            Self::AddImage(_) => "add_image",
            Self::ReplaceImage(_) => "replace_image",
        }
    }
}

impl ReplaceTextOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        replace_text_target_selector(self)
    }

    #[must_use]
    pub fn run_selector(&self) -> Option<&RunSelector> {
        match (&self.selector, &self.run) {
            (Some(Selector::ElementId { run: Some(run), .. }), _) => Some(run),
            (_, Some(run)) => Some(run),
            _ => None,
        }
    }

    #[must_use]
    pub fn target_element_id(&self) -> &str {
        target_element_id(&self.element_id, self.selector.as_ref()).unwrap_or(&self.element_id)
    }

    #[must_use]
    pub fn target_slide_id(&self) -> &str {
        target_slide_id(&self.slide_id, self.selector.as_ref()).unwrap_or(&self.slide_id)
    }
}

impl AddTextBoxOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        slide_target_selector(&self.operation_id, &self.slide_id, self.selector.as_ref())
    }

    #[must_use]
    pub fn target_slide_id(&self) -> &str {
        target_slide_id(&self.slide_id, self.selector.as_ref()).unwrap_or(&self.slide_id)
    }
}

impl MoveResizeElementOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        element_target_selector(&self.operation_id, &self.element_id, self.selector.as_ref())
    }

    #[must_use]
    pub fn target_element_id(&self) -> &str {
        target_element_id(&self.element_id, self.selector.as_ref()).unwrap_or(&self.element_id)
    }
}

impl SetAltTextOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        element_target_selector(&self.operation_id, &self.element_id, self.selector.as_ref())
    }

    #[must_use]
    pub fn target_element_id(&self) -> &str {
        target_element_id(&self.element_id, self.selector.as_ref()).unwrap_or(&self.element_id)
    }
}

impl AddImageOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        slide_target_selector(&self.operation_id, &self.slide_id, self.selector.as_ref())
    }

    #[must_use]
    pub fn target_slide_id(&self) -> &str {
        target_slide_id(&self.slide_id, self.selector.as_ref()).unwrap_or(&self.slide_id)
    }
}

impl ReplaceImageOperation {
    pub fn target_selector(&self) -> Result<Selector> {
        element_target_selector(&self.operation_id, &self.element_id, self.selector.as_ref())
    }

    #[must_use]
    pub fn target_element_id(&self) -> &str {
        target_element_id(&self.element_id, self.selector.as_ref()).unwrap_or(&self.element_id)
    }
}

impl SetDocumentMetadataOperation {
    pub fn target_selector(&self) -> Selector {
        self.selector.clone()
    }
}

fn element_target_selector(
    operation_id: &str,
    shorthand: &str,
    selector: Option<&Selector>,
) -> Result<Selector> {
    match selector {
        Some(Selector::ElementId { id, .. }) if shorthand.is_empty() || shorthand == id => {
            selector.cloned().ok_or_else(|| {
                selector_conflict(
                    operation_id,
                    None,
                    "Operation must include either element_id shorthand or an element_id selector.",
                )
            })
        }
        Some(Selector::ElementId { id, .. }) => Err(selector_conflict(
            operation_id,
            Some(shorthand),
            format!(
                "Operation target conflict: element_id `{shorthand}` does not match selector id `{id}`."
            ),
        )),
        Some(
            Selector::SlideId { .. } | Selector::MediaPart { .. } | Selector::CoreProperties { .. },
        ) => Err(selector_conflict(
            operation_id,
            target_element_id(shorthand, selector),
            "Operation selector must have type `element_id` for an element-targeting operation.",
        )),
        None if shorthand.is_empty() => Err(selector_conflict(
            operation_id,
            None,
            "Operation must include either element_id shorthand or an element_id selector.",
        )),
        None => Ok(Selector::ElementId {
            id: shorthand.to_owned(),
            guards: None,
            run: None,
        }),
    }
}

fn slide_target_selector(
    operation_id: &str,
    shorthand: &str,
    selector: Option<&Selector>,
) -> Result<Selector> {
    match selector {
        Some(Selector::SlideId { id, .. }) if shorthand.is_empty() || shorthand == id => {
            selector.cloned().ok_or_else(|| {
                selector_conflict(
                    operation_id,
                    None,
                    "Operation must include either slide_id shorthand or a slide_id selector.",
                )
            })
        }
        Some(Selector::SlideId { id, .. }) => Err(selector_conflict(
            operation_id,
            None,
            format!(
                "Operation target conflict: slide_id `{shorthand}` does not match selector id `{id}`."
            ),
        )),
        Some(
            Selector::ElementId { .. }
            | Selector::MediaPart { .. }
            | Selector::CoreProperties { .. },
        ) => Err(selector_conflict(
            operation_id,
            target_element_id(shorthand, selector),
            "Operation selector must have type `slide_id` for a slide-targeting operation.",
        )),
        None if shorthand.is_empty() => Err(selector_conflict(
            operation_id,
            None,
            "Operation must include either slide_id shorthand or a slide_id selector.",
        )),
        None => Ok(Selector::SlideId {
            id: shorthand.to_owned(),
            guards: None,
        }),
    }
}

fn replace_text_target_selector(operation: &ReplaceTextOperation) -> Result<Selector> {
    match operation.selector.as_ref() {
        Some(Selector::ElementId { .. }) if operation.slide_id.is_empty() => {
            element_target_selector(
                &operation.operation_id,
                &operation.element_id,
                operation.selector.as_ref(),
            )
        }
        Some(Selector::ElementId { .. }) => Err(selector_conflict(
            &operation.operation_id,
            target_element_id(&operation.element_id, operation.selector.as_ref()),
            "replace_text cannot combine an element_id selector with slide_id shorthand.",
        )),
        Some(Selector::SlideId { .. })
            if operation.element_id.is_empty() && operation.cell.is_none() =>
        {
            slide_target_selector(
                &operation.operation_id,
                &operation.slide_id,
                operation.selector.as_ref(),
            )
        }
        Some(Selector::SlideId { .. }) => Err(selector_conflict(
            &operation.operation_id,
            None,
            "replace_text cannot combine a slide_id selector with element_id shorthand or cell.",
        )),
        Some(Selector::MediaPart { .. } | Selector::CoreProperties { .. }) => {
            Err(selector_conflict(
                &operation.operation_id,
                target_element_id(&operation.element_id, operation.selector.as_ref()),
                "replace_text selector must have type `element_id` or `slide_id`.",
            ))
        }
        None if !operation.slide_id.is_empty() && !operation.element_id.is_empty() => {
            Err(selector_conflict(
                &operation.operation_id,
                Some(&operation.element_id),
                "replace_text must target either slide_id notes or element_id content, not both.",
            ))
        }
        None if !operation.slide_id.is_empty() && operation.cell.is_some() => {
            Err(selector_conflict(
                &operation.operation_id,
                None,
                "replace_text cannot combine slide_id notes with cell.",
            ))
        }
        None if !operation.slide_id.is_empty() => {
            slide_target_selector(&operation.operation_id, &operation.slide_id, None)
        }
        None => element_target_selector(&operation.operation_id, &operation.element_id, None),
    }
}

fn target_element_id<'a>(shorthand: &'a str, selector: Option<&'a Selector>) -> Option<&'a str> {
    if !shorthand.is_empty() {
        return Some(shorthand);
    }
    match selector {
        Some(Selector::ElementId { id, .. }) => Some(id),
        Some(
            Selector::SlideId { .. } | Selector::MediaPart { .. } | Selector::CoreProperties { .. },
        )
        | None => None,
    }
}

fn target_slide_id<'a>(shorthand: &'a str, selector: Option<&'a Selector>) -> Option<&'a str> {
    if !shorthand.is_empty() {
        return Some(shorthand);
    }
    match selector {
        Some(Selector::SlideId { id, .. }) => Some(id),
        Some(
            Selector::ElementId { .. }
            | Selector::MediaPart { .. }
            | Selector::CoreProperties { .. },
        )
        | None => None,
    }
}

fn selector_conflict(
    operation_id: &str,
    element_id: Option<&str>,
    message: impl Into<String>,
) -> Error {
    Error::new(ErrorCode::InvalidInput, message).with_location(ErrorLocation {
        operation_id: Some(operation_id.to_owned()),
        element_id: element_id.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceTextOperation {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target element shorthand for slide shape/table text. Exactly one target is required: element_id, slide_id, or selector."
    )]
    pub element_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target slide shorthand for speaker-notes text. Exactly one target is required: element_id, slide_id, or selector."
    )]
    pub slide_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For replace_text this must be type element_id for slide content or slide_id for speaker notes."
    )]
    pub selector: Option<Selector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Optional table-cell coordinate. When present, replace_text targets this cell within the selected table graphic-frame element."
    )]
    pub cell: Option<TableCellSelector>,
    pub text: String,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub current_text_match: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ReplaceTextMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_policy: Option<FormatPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_policy: Option<OverflowPolicy>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_formatting_simplification: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Optional run-property overrides accepted only when mode is run_scoped. Supports font_size_pt, bold, italic, color_hex, font_family, and paragraph-level align."
    )]
    pub run_style: Option<TextBoxStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Run selector for speaker-notes or table-cell replacements. Element text may alternatively carry run selection inside selector.run."
    )]
    pub run: Option<RunSelector>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableCellSelector {
    pub row: u32,
    pub col: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceTextMode {
    WholeElement,
    RunScoped,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target slide shorthand. Either slide_id or selector is required; when both are present, they must identify the same slide."
    )]
    pub slide_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For add_text_box this must be type slide_id."
    )]
    pub selector: Option<Selector>,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target element shorthand. Either element_id or selector is required; when both are present, they must identify the same element."
    )]
    pub element_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For move_resize_element this must be type element_id."
    )]
    pub selector: Option<Selector>,
    pub bounds: Bounds,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAltTextOperation {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target element shorthand. Either element_id or selector is required; when both are present, they must identify the same element."
    )]
    pub element_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For set_alt_text this must be type element_id."
    )]
    pub selector: Option<Selector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetDocumentMetadataOperation {
    pub operation_id: String,
    #[schemars(
        description = "Canonical core-properties selector resolved through the package root core-properties relationship."
    )]
    pub selector: Selector,
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub current_value_match: Option<DocumentMetadataFields>,
    pub metadata: DocumentMetadataFields,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentMetadataFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
}

impl DocumentMetadataFields {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.subject.is_none()
            && self.creator.is_none()
            && self.keywords.is_none()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddImageOperation {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target slide shorthand. Either slide_id or selector is required; when both are present, they must identify the same slide."
    )]
    pub slide_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For add_image this must be type slide_id."
    )]
    pub selector: Option<Selector>,
    pub media_ref: String,
    pub content_type: String,
    pub bounds: Bounds,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert: Option<InsertOptions>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceImageOperation {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schemars(
        description = "Target picture element shorthand. Either element_id or selector is required; when both are present, they must identify the same element."
    )]
    pub element_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Canonical target selector with optional guards. For replace_image this must be type element_id."
    )]
    pub selector: Option<Selector>,
    pub media_ref: String,
    pub content_type: String,
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
#[serde(deny_unknown_fields)]
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
pub enum ZOrderKeyword {
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ZOrder {
    Keyword(ZOrderKeyword),
    Index(u32),
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

#[derive(Clone, Debug, PartialEq)]
pub struct StagedPatchResult {
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
            client_request_id: None,
            request_id: None,
            transaction_id: None,
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
            warnings: Vec::new(),
            validation: patch_validation_summary(&validation),
        },
    })
}

pub fn apply_patch<E>(
    package: &mut Package,
    context: PatchContext,
    patch: &Patch,
    dry_run: bool,
    executor: &mut E,
) -> Result<PatchReport>
where
    E: OperationExecutor,
{
    let result = apply_patch_staged(package, context, patch, dry_run, executor)?;
    if !dry_run && result.report.status == PatchStatus::Applied {
        *package = result.package.into_inner();
    }
    Ok(result.report)
}

pub fn apply_patch_staged<E>(
    package: &Package,
    mut context: PatchContext,
    patch: &Patch,
    dry_run: bool,
    executor: &mut E,
) -> Result<StagedPatchResult>
where
    E: OperationExecutor,
{
    context.dry_run = dry_run;
    let mut staged = package.clone();
    let mut operation_reports = Vec::with_capacity(patch.operations.len());
    let mut changed_parts = Vec::new();
    let mut failed = false;

    for operation in &patch.operations {
        let effects = match executor.validate(&staged, operation) {
            Ok(_validation_effects) => match executor.apply(&mut staged, operation) {
                Ok(effects) => effects,
                Err(error) if dry_run => {
                    failed = true;
                    operation_reports.push(failed_operation_report(operation, &error)?);
                    continue;
                }
                Err(error) => {
                    failed = true;
                    operation_reports.push(failed_operation_report(operation, &error)?);
                    continue;
                }
            },
            Err(error) if dry_run => {
                failed = true;
                operation_reports.push(failed_operation_report(operation, &error)?);
                continue;
            }
            Err(error) => {
                failed = true;
                operation_reports.push(failed_operation_report(operation, &error)?);
                continue;
            }
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

    let report_changed_parts = if failed { Vec::new() } else { changed_parts };
    let wrote_part = !report_changed_parts.is_empty();
    let report_revision = if failed || dry_run || !wrote_part {
        context.base_revision
    } else {
        context.new_revision
    };
    let validation_package = if failed { package } else { &staged };
    let validation = validate_for_write(
        validation_package,
        &context.new_document_id,
        report_revision,
    )?;

    if has_blocking_findings(&validation) {
        return Err(validation_failed(validation));
    }

    Ok(StagedPatchResult {
        package: WritablePackage { package: staged },
        report: PatchReport {
            schema: PATCH_REPORT_SCHEMA.to_owned(),
            version: PATCH_REPORT_VERSION,
            client_request_id: Some(patch.client_request_id.clone()),
            request_id: None,
            transaction_id: None,
            status: if failed {
                if dry_run {
                    PatchStatus::DryRunFailed
                } else {
                    PatchStatus::Failed
                }
            } else if dry_run {
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
            changed_parts: report_changed_parts,
            warnings: Vec::new(),
            validation: patch_validation_summary(&validation),
        },
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
        error: None,
    }
}

fn failed_operation_report(operation: &Operation, error: &Error) -> Result<OperationReport> {
    Ok(OperationReport {
        operation_id: operation.operation_id().to_owned(),
        op: operation.op_name().to_owned(),
        status: OperationStatus::Failed,
        target: target_from_error(error),
        changed_parts: Vec::new(),
        created_element_ids: Vec::new(),
        warnings: Vec::new(),
        error: Some(error_view(error)?),
    })
}

fn target_from_error(error: &Error) -> OperationTarget {
    let location = &error.details().location;
    OperationTarget {
        slide_id: location.slide_id.clone().unwrap_or_default(),
        element_id: location.element_id.clone().unwrap_or_default(),
        part: location.part.clone().unwrap_or_default(),
    }
}

fn error_view(error: &Error) -> Result<ErrorView> {
    let details = error.details();
    Ok(ErrorView {
        code: error_code(details.code),
        message: details.message.clone(),
        severity: error_severity(details.severity),
        category: serde_json::to_value(details.category)
            .map_err(|source| {
                Error::with_source(
                    ErrorCode::InternalError,
                    "Could not serialize operation error category.",
                    source,
                )
            })?
            .as_str()
            .unwrap_or("internal")
            .to_owned(),
        retryable: details.retryable,
        state_changed: details.state_changed,
        location: serde_json::to_value(&details.location).map_err(|source| {
            Error::with_source(
                ErrorCode::InternalError,
                "Could not serialize operation error location.",
                source,
            )
        })?,
        suggestions: details.suggestions.clone(),
    })
}

const fn error_code(code: ErrorCode) -> pptx_compose_json::schemas::ErrorCode {
    match code {
        ErrorCode::InvalidInput => pptx_compose_json::schemas::ErrorCode::InvalidInput,
        ErrorCode::UnsafePath => pptx_compose_json::schemas::ErrorCode::UnsafePath,
        ErrorCode::ResourceLimitExceeded => {
            pptx_compose_json::schemas::ErrorCode::ResourceLimitExceeded
        }
        ErrorCode::UnsupportedPackage => pptx_compose_json::schemas::ErrorCode::UnsupportedPackage,
        ErrorCode::UnsupportedEdit => pptx_compose_json::schemas::ErrorCode::UnsupportedEdit,
        ErrorCode::UnsupportedMediaType => {
            pptx_compose_json::schemas::ErrorCode::UnsupportedMediaType
        }
        ErrorCode::InvalidBounds => pptx_compose_json::schemas::ErrorCode::InvalidBounds,
        ErrorCode::ParseError => pptx_compose_json::schemas::ErrorCode::ParseError,
        ErrorCode::MalformedXml => pptx_compose_json::schemas::ErrorCode::MalformedXml,
        ErrorCode::ValidationFailed => pptx_compose_json::schemas::ErrorCode::ValidationFailed,
        ErrorCode::StalePatch => pptx_compose_json::schemas::ErrorCode::StalePatch,
        ErrorCode::SelectorNotFound => pptx_compose_json::schemas::ErrorCode::SelectorNotFound,
        ErrorCode::SelectorAmbiguous => pptx_compose_json::schemas::ErrorCode::SelectorAmbiguous,
        ErrorCode::SelectorGuardFailed => {
            pptx_compose_json::schemas::ErrorCode::SelectorGuardFailed
        }
        ErrorCode::MissingMediaRef => pptx_compose_json::schemas::ErrorCode::MissingMediaRef,
        ErrorCode::MediaChecksumMismatch => {
            pptx_compose_json::schemas::ErrorCode::MediaChecksumMismatch
        }
        ErrorCode::PermissionDenied => pptx_compose_json::schemas::ErrorCode::PermissionDenied,
        ErrorCode::WriteFailed => pptx_compose_json::schemas::ErrorCode::WriteFailed,
        ErrorCode::InternalError => pptx_compose_json::schemas::ErrorCode::InternalError,
    }
}

const fn error_severity(
    severity: pptx_compose_core::error::ErrorSeverity,
) -> pptx_compose_json::schemas::Severity {
    match severity {
        pptx_compose_core::error::ErrorSeverity::Info => pptx_compose_json::schemas::Severity::Info,
        pptx_compose_core::error::ErrorSeverity::Warning => {
            pptx_compose_json::schemas::Severity::Warning
        }
        pptx_compose_core::error::ErrorSeverity::Error => {
            pptx_compose_json::schemas::Severity::Error
        }
        pptx_compose_core::error::ErrorSeverity::Fatal => {
            pptx_compose_json::schemas::Severity::Fatal
        }
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
fn all_op_names_matches_operation_variants() {
    let bounds = Bounds {
        x: 1,
        y: 2,
        cx: 3,
        cy: 4,
    };
    let variants = [
        Operation::ReplaceText(ReplaceTextOperation {
            operation_id: "op-1".to_owned(),
            element_id: "slide-1:shape-1".to_owned(),
            slide_id: String::new(),
            selector: None,
            cell: None,
            text: "updated".to_owned(),
            current_text_match: None,
            mode: None,
            format_policy: None,
            overflow_policy: None,
            allow_formatting_simplification: false,
            run_style: None,
            run: None,
        }),
        Operation::AddTextBox(AddTextBoxOperation {
            operation_id: "op-2".to_owned(),
            slide_id: "slide-1".to_owned(),
            selector: None,
            text: "new".to_owned(),
            bounds: bounds.clone(),
            name: None,
            alt_text: None,
            style: None,
            insert: None,
        }),
        Operation::MoveResizeElement(MoveResizeElementOperation {
            operation_id: "op-3".to_owned(),
            element_id: "slide-1:shape-1".to_owned(),
            selector: None,
            bounds: bounds.clone(),
        }),
        Operation::SetAltText(SetAltTextOperation {
            operation_id: "op-4".to_owned(),
            element_id: "slide-1:shape-1".to_owned(),
            selector: None,
            title: None,
            description: Some("description".to_owned()),
        }),
        Operation::SetDocumentMetadata(SetDocumentMetadataOperation {
            operation_id: "op-5".to_owned(),
            selector: Selector::CoreProperties {
                part: "docProps/core.xml".to_owned(),
                guards: None,
            },
            current_value_match: None,
            metadata: DocumentMetadataFields {
                title: Some("Title".to_owned()),
                ..DocumentMetadataFields::default()
            },
        }),
        Operation::AddImage(AddImageOperation {
            operation_id: "op-6".to_owned(),
            slide_id: "slide-1".to_owned(),
            selector: None,
            media_ref: "image-1".to_owned(),
            content_type: "image/png".to_owned(),
            bounds,
            name: None,
            alt_text: None,
            insert: None,
        }),
        Operation::ReplaceImage(ReplaceImageOperation {
            operation_id: "op-7".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            selector: None,
            media_ref: "image-1".to_owned(),
            content_type: "image/png".to_owned(),
        }),
    ];
    let variant_names = variants.map(|operation| operation.op_name());

    assert_eq!(variant_names, ALL_OP_NAMES);
}

#[cfg(test)]
#[test]
fn add_image_rejects_unimplemented_fit_and_dedupe_fields() {
    for field in ["fit", "dedupe"] {
        let mut operation = serde_json::json!({
            "operation_id": format!("add-image-{field}"),
            "op": "add_image",
            "slide_id": "slide-1",
            "media_ref": "image-1",
            "content_type": "image/png",
            "bounds": { "x": 1, "y": 2, "cx": 3, "cy": 4 }
        });
        operation
            .as_object_mut()
            .expect("operation is an object")
            .insert(field.to_owned(), serde_json::json!("stretch"));

        let error = parse_patch(serde_json::json!({
            "schema": PATCH_SCHEMA,
            "version": PATCH_VERSION,
            "document_id": "sha256:current",
            "base_revision": 3,
            "client_request_id": format!("agent-run-{field}"),
            "operations": [operation]
        }))
        .expect_err("unimplemented add_image schema field is rejected");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
    }
}

#[cfg(test)]
#[test]
fn selector_object_targets_parse_and_detect_shorthand_conflicts() {
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
                "selector": {
                    "type": "element_id",
                    "id": "slide-1:shape-4",
                    "guards": {
                        "slide_id": "slide-1",
                        "kind": "text_box",
                        "part": "ppt/slides/slide1.xml",
                        "text_hash": "sha256:text",
                        "fingerprint": "sha256:fingerprint"
                    }
                },
                "text": "Updated title"
            }
        ]
    }))
    .expect("selector-only target parses");

    let Operation::ReplaceText(operation) = &patch.operations[0] else {
        panic!("operation type");
    };
    assert_eq!(operation.target_element_id(), "slide-1:shape-4");
    assert!(matches!(
        operation.target_selector().expect("selector resolves"),
        Selector::ElementId { .. }
    ));

    let conflict = parse_patch(serde_json::json!({
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
                "selector": {
                    "type": "element_id",
                    "id": "slide-1:shape-5"
                },
                "text": "Updated title"
            }
        ]
    }))
    .expect("conflicting target parses for semantic validation");
    let Operation::ReplaceText(operation) = &conflict.operations[0] else {
        panic!("operation type");
    };
    let error = operation
        .target_selector()
        .expect_err("conflicting shorthand and selector fails");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert_eq!(
        error.details().location.operation_id.as_deref(),
        Some("op-1")
    );
}

#[cfg(test)]
#[test]
fn patch_schema_documents_selector_targets() {
    let schema = serde_json::to_value(schemars::schema_for!(Patch)).expect("schema serializes");
    let schema_text = serde_json::to_string(&schema).expect("schema text serializes");

    assert!(schema_text.contains("\"selector\""));
    assert!(schema_text.contains("Canonical target selector with optional guards"));
    assert!(schema_text.contains("Either element_id or selector is required"));
    assert!(schema_text.contains("\"element_id\""));
    assert!(schema_text.contains("\"slide_id\""));
    assert!(schema_text.contains("\"media_part\""));
}

#[cfg(test)]
#[test]
fn apply_is_atomic() {
    test_support::apply_is_atomic();
}

#[cfg(test)]
#[test]
fn dry_run_reports_all_failed_operations() {
    test_support::dry_run_reports_all_failed_operations();
}

#[cfg(test)]
#[test]
fn non_dry_run_reports_failed_operation_without_committing() {
    test_support::non_dry_run_reports_failed_operation_without_committing();
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

        let report = super::apply_patch(&mut package, context(), &patch, false, &mut executor)
            .expect("second operation failure returns a patch report");

        assert_eq!(
            report.status,
            pptx_compose_json::schemas::PatchStatus::Failed
        );
        assert_eq!(report.changed_parts, Vec::<String>::new());
        assert_eq!(report.new_revision, report.base_revision);
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

    pub fn dry_run_reports_all_failed_operations() {
        let mut package = base_package();
        let original = package.clone();
        let patch = two_op_patch();
        let mut executor = FailingAllOps;

        let report = super::apply_patch(&mut package, context(), &patch, true, &mut executor)
            .expect("dry-run returns a failure report");

        assert_eq!(
            report.status,
            pptx_compose_json::schemas::PatchStatus::DryRunFailed
        );
        assert_eq!(report.operation_reports.len(), 2);
        assert!(
            report
                .operation_reports
                .iter()
                .all(|operation| operation.status == OperationStatus::Failed)
        );
        assert_eq!(
            report
                .operation_reports
                .iter()
                .filter_map(|operation| operation.error.as_ref())
                .filter(|error| {
                    error.code == pptx_compose_json::schemas::ErrorCode::UnsupportedEdit
                })
                .count(),
            2
        );
        assert_eq!(package, original);
    }

    pub fn non_dry_run_reports_failed_operation_without_committing() {
        let mut package = base_package();
        let original = package.clone();
        let patch = two_op_patch();
        let mut executor = FailingSecondOp;

        let report = super::apply_patch(&mut package, context(), &patch, false, &mut executor)
            .expect("non-dry-run failure returns a report");

        assert_eq!(
            report.status,
            pptx_compose_json::schemas::PatchStatus::Failed
        );
        assert!(!report.dry_run);
        assert_eq!(report.changed_parts, Vec::<String>::new());
        assert_eq!(report.new_revision, report.base_revision);
        assert_eq!(report.operation_reports.len(), 2);
        assert_eq!(report.operation_reports[0].operation_id, "op-1");
        assert_eq!(report.operation_reports[0].status, OperationStatus::Applied);
        assert_eq!(report.operation_reports[1].operation_id, "op-2");
        assert_eq!(report.operation_reports[1].status, OperationStatus::Failed);
        assert_eq!(
            report.operation_reports[1]
                .error
                .as_ref()
                .expect("failed operation reports an error")
                .code,
            pptx_compose_json::schemas::ErrorCode::UnsupportedEdit
        );
        assert_eq!(package, original);
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

    struct FailingAllOps;

    impl OperationExecutor for FailingAllOps {
        fn validate(&mut self, _package: &Package, operation: &Operation) -> Result<PatchEffects> {
            Err(Error::new(
                ErrorCode::UnsupportedEdit,
                format!(
                    "Test operation {} is unsupported.",
                    operation.operation_id()
                ),
            ))
        }

        fn apply(
            &mut self,
            _package: &mut Package,
            _operation: &Operation,
        ) -> Result<PatchEffects> {
            unreachable!("failed validation prevents apply")
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
                    selector: None,
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
                    selector: None,
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
            .insert_zip_entry(
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#
                    .to_vec(),
            )
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
