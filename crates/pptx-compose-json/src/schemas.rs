use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_view::AgentView;
use crate::schema_versions::{
    AGENT_VIEW_SCHEMA, ERROR_SCHEMA, ERROR_VERSION, PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION,
    RESULT_SCHEMA, RESULT_VERSION, VALIDATION_REPORT_SCHEMA, VALIDATION_REPORT_VERSION,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultEnvelope {
    #[serde(default = "result_schema")]
    pub schema: String,
    #[serde(default = "result_version")]
    pub version: u32,
    pub status: ResultStatus,
    pub result: Value,
    pub warnings: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Success,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchReport {
    #[serde(default = "patch_report_schema")]
    pub schema: String,
    #[serde(default = "patch_report_version")]
    pub version: u32,
    pub status: PatchStatus,
    pub dry_run: bool,
    pub document_id: String,
    pub base_revision: u32,
    pub new_document_id: String,
    pub new_revision: u32,
    pub operation_reports: Vec<OperationReport>,
    pub changed_parts: Vec<String>,
    pub validation: PatchValidationSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatchStatus {
    DryRunSuccess,
    DryRunFailed,
    Applied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationReport {
    pub operation_id: String,
    pub op: String,
    pub status: OperationStatus,
    pub target: OperationTarget,
    pub changed_parts: Vec<String>,
    pub created_element_ids: Vec<String>,
    pub warnings: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Validated,
    Applied,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationTarget {
    pub slide_id: String,
    pub element_id: String,
    pub part: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchValidationSummary {
    pub status: ValidationStatus,
    pub errors: u32,
    pub warnings: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationReport {
    #[serde(default = "validation_report_schema")]
    pub schema: String,
    #[serde(default = "validation_report_version")]
    pub version: u32,
    pub document_id: String,
    pub revision: u32,
    pub status: ValidationStatus,
    pub summary: Summary,
    pub findings: Vec<FindingView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub fatal: u32,
    pub errors: u32,
    pub warnings: u32,
    pub info: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingView {
    pub id: String,
    pub severity: Severity,
    pub category: FindingCategory,
    pub code: FindingCode,
    pub message: String,
    pub blocking: bool,
    pub location: Value,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    ContentType,
    Relationship,
    Presentation,
    Slide,
    Xml,
    Package,
    Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    MissingContentType,
    MediaContentTypeMismatch,
    DanglingInternalRelationship,
    UnresolvedRelationshipReference,
    DuplicateRelationshipId,
    ExternalRelationshipNotChecked,
    DuplicateSlideId,
    SlideOrderMismatch,
    DuplicateDrawingId,
    InvalidBounds,
    MalformedXml,
    MissingNamespaceDeclaration,
    PartDropped,
    OrphanPart,
    SignatureInvalidatedByEdit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    #[serde(default = "error_schema")]
    pub schema: String,
    #[serde(default = "error_version")]
    pub version: u32,
    pub status: ErrorStatus,
    pub error: ErrorView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStatus {
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorView {
    pub code: ErrorCode,
    pub message: String,
    pub severity: Severity,
    pub category: String,
    pub retryable: bool,
    pub state_changed: bool,
    pub location: Value,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    UnsafePath,
    ResourceLimitExceeded,
    UnsupportedPackage,
    UnsupportedEdit,
    UnsupportedMediaType,
    InvalidBounds,
    ParseError,
    ValidationFailed,
    StalePatch,
    SelectorNotFound,
    SelectorAmbiguous,
    SelectorGuardFailed,
    MissingMediaRef,
    MediaChecksumMismatch,
    PermissionDenied,
    WriteFailed,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    SerializeSchema(String),
}

pub fn agent_view_json_schema() -> Result<Value, JsonError> {
    let schema = schemars::schema_for!(AgentView);
    let mut value =
        serde_json::to_value(schema).map_err(|err| JsonError::SerializeSchema(err.to_string()))?;

    if let Some(object) = value.as_object_mut() {
        object.insert(
            "$id".to_owned(),
            Value::String(AGENT_VIEW_SCHEMA.to_owned()),
        );
    }

    Ok(value)
}

fn result_schema() -> String {
    RESULT_SCHEMA.to_owned()
}

const fn result_version() -> u32 {
    RESULT_VERSION
}

fn patch_report_schema() -> String {
    PATCH_REPORT_SCHEMA.to_owned()
}

const fn patch_report_version() -> u32 {
    PATCH_REPORT_VERSION
}

fn validation_report_schema() -> String {
    VALIDATION_REPORT_SCHEMA.to_owned()
}

const fn validation_report_version() -> u32 {
    VALIDATION_REPORT_VERSION
}

fn error_schema() -> String {
    ERROR_SCHEMA.to_owned()
}

const fn error_version() -> u32 {
    ERROR_VERSION
}

#[cfg(test)]
#[test]
fn roundtrips_044_examples() {
    assert_roundtrip::<ResultEnvelope>(
        r#"{
          "schema": "pptx-compose.result.v1",
          "version": 1,
          "status": "success",
          "result": {},
          "warnings": [],
          "next_cursor": null
        }"#,
    );

    assert_roundtrip::<PatchReport>(
        r#"{
          "schema": "pptx-compose.patch_report.v1",
          "version": 1,
          "status": "applied",
          "dry_run": false,
          "document_id": "sha256:old",
          "base_revision": 1,
          "new_document_id": "sha256:new",
          "new_revision": 2,
          "operation_reports": [
            {
              "operation_id": "op-1",
              "op": "replace_text",
              "status": "applied",
              "target": {
                "slide_id": "slide-1",
                "element_id": "slide-1:shape-4",
                "part": "ppt/slides/slide1.xml"
              },
              "changed_parts": ["ppt/slides/slide1.xml"],
              "created_element_ids": [],
              "warnings": []
            }
          ],
          "changed_parts": ["ppt/slides/slide1.xml"],
          "validation": { "status": "valid", "errors": 0, "warnings": 0 }
        }"#,
    );

    assert_roundtrip::<ValidationReport>(
        r#"{
          "schema": "pptx-compose.validation_report.v1",
          "version": 1,
          "document_id": "sha256:...",
          "revision": 1,
          "status": "valid",
          "summary": { "fatal": 0, "errors": 0, "warnings": 1, "info": 3 },
          "findings": [
            {
              "id": "finding-1",
              "severity": "warning",
              "category": "relationship",
              "code": "external_relationship_not_checked",
              "message": "External relationship was preserved but not fetched",
              "blocking": false,
              "location": { "part": "ppt/slides/slide1.xml", "relationship_id": "rId5" },
              "suggested_action": null
            }
          ]
        }"#,
    );

    assert_roundtrip::<ErrorEnvelope>(
        r#"{
          "schema": "pptx-compose.error.v1",
          "version": 1,
          "status": "error",
          "error": {
            "code": "stale_patch",
            "message": "Patch base_revision does not match current revision.",
            "severity": "error",
            "category": "patch",
            "retryable": false,
            "state_changed": false,
            "location": { "operation_id": "op-1", "element_id": "slide-1:shape-4" },
            "suggestions": ["Inspect the deck again and regenerate the patch."]
          }
        }"#,
    );
}

#[cfg(test)]
fn assert_roundtrip<T>(json: &str)
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let input: Value = serde_json::from_str(json).expect("044 example is valid JSON");
    let envelope: T = serde_json::from_value(input.clone()).expect("044 example deserializes");
    let output = serde_json::to_value(envelope).expect("044 example re-serializes");

    assert_eq!(output, input);
}
