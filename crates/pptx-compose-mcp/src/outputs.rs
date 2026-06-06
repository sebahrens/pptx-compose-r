use pptx_compose::core::error::{Error as CoreError, ErrorCategory, ErrorSeverity};
use pptx_compose::json::{
    schema_versions::{ERROR_SCHEMA, ERROR_VERSION, RESULT_SCHEMA, RESULT_VERSION},
    schemas::{ResultEnvelope, ResultStatus},
};
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

macro_rules! result_output {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub ResultEnvelope);
    };
}

result_output!(OpenOutput);
result_output!(DocumentSummaryOutput);
result_output!(ListSlidesOutput);
result_output!(SlideOutput);
result_output!(ListElementsOutput);
result_output!(ElementOutput);
result_output!(FindTextOutput);
result_output!(ImportMediaOutput);
result_output!(ValidatePatchOutput);
result_output!(ApplyPatchOutput);
result_output!(ValidateOutput);
result_output!(ExportOutput);
result_output!(CloseOutput);
result_output!(RawPartXmlOutput);
result_output!(ReplacePartXmlOutput);

impl DocumentSummaryOutput {
    #[must_use]
    pub fn stub(tool: &str) -> Self {
        Self(stub_envelope(tool))
    }
}

macro_rules! impl_stub {
    ($($name:ident),+ $(,)?) => {
        $(
            impl $name {
                #[must_use]
                pub fn stub(tool: &str) -> Self {
                    Self(stub_envelope(tool))
                }
            }
        )+
    };
}

impl_stub!(
    OpenOutput,
    ListSlidesOutput,
    SlideOutput,
    ListElementsOutput,
    ElementOutput,
    FindTextOutput,
    ImportMediaOutput,
    ValidatePatchOutput,
    ApplyPatchOutput,
    ValidateOutput,
    ExportOutput,
    CloseOutput,
    RawPartXmlOutput,
    ReplacePartXmlOutput,
);

impl OpenOutput {
    #[must_use]
    pub fn opened(session: crate::sessions::OpenSession) -> Self {
        Self(success_envelope(json!(session)))
    }
}

impl ImportMediaOutput {
    #[must_use]
    pub fn imported(handle: crate::sessions::MediaHandle) -> Self {
        Self(success_envelope(json!(handle)))
    }
}

impl FindTextOutput {
    #[must_use]
    pub fn found(result: pptx_compose::json::agent_view::FindTextResult) -> Self {
        Self(success_envelope(json!(result)))
    }
}

impl ApplyPatchOutput {
    #[must_use]
    pub fn applied(session_id: &str, revision: u64, dry_run: bool) -> Self {
        Self(success_envelope(json!({
            "session_id": session_id,
            "revision": revision,
            "dry_run": dry_run
        })))
    }
}

impl CloseOutput {
    #[must_use]
    pub fn closed(session_id: &str, closed: bool) -> Self {
        Self(success_envelope(json!({
            "session_id": session_id,
            "closed": closed
        })))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub schema: String,
    pub version: u32,
    pub status: ErrorStatus,
    pub error: ErrorView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStatus {
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorView {
    pub code: String,
    pub message: String,
    pub severity: String,
    pub category: String,
    pub retryable: bool,
    pub state_changed: bool,
    pub location: Value,
    pub suggestions: Vec<String>,
}

#[must_use]
pub fn map_error(error: CoreError) -> CallToolResult {
    let envelope = error_envelope(&error);
    let value = serde_json::to_value(envelope).unwrap_or_else(|serialize_error| {
        json!({
            "schema": ERROR_SCHEMA,
            "version": ERROR_VERSION,
            "status": "error",
            "error": {
                "code": "internal_error",
                "message": format!("Could not serialize MCP error envelope: {serialize_error}."),
                "severity": "error",
                "category": "internal",
                "retryable": false,
                "state_changed": false,
                "location": {},
                "suggestions": []
            }
        })
    });

    CallToolResult::structured_error(value)
}

#[must_use]
pub fn error_envelope(error: &CoreError) -> ErrorEnvelope {
    let details = error.details();
    ErrorEnvelope {
        schema: ERROR_SCHEMA.to_owned(),
        version: ERROR_VERSION,
        status: ErrorStatus::Error,
        error: ErrorView {
            code: details.code.as_str().to_owned(),
            message: details.message.clone(),
            severity: severity(details.severity).to_owned(),
            category: category(details.category).to_owned(),
            retryable: details.retryable,
            state_changed: details.state_changed,
            location: serde_json::to_value(&details.location).unwrap_or_else(|_| json!({})),
            suggestions: details.suggestions.clone(),
        },
    }
}

fn stub_envelope(tool: &str) -> ResultEnvelope {
    success_envelope(json!({
        "status": "stub",
        "tool": tool,
    }))
}

fn success_envelope(result: Value) -> ResultEnvelope {
    ResultEnvelope {
        schema: RESULT_SCHEMA.to_owned(),
        version: RESULT_VERSION,
        status: ResultStatus::Success,
        result,
        warnings: Vec::new(),
        next_cursor: None,
    }
}

const fn severity(severity: ErrorSeverity) -> &'static str {
    match severity {
        ErrorSeverity::Info => "info",
        ErrorSeverity::Warning => "warning",
        ErrorSeverity::Error => "error",
        ErrorSeverity::Fatal => "fatal",
    }
}

const fn category(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Input => "input",
        ErrorCategory::Path => "path",
        ErrorCategory::Resource => "resource",
        ErrorCategory::Package => "package",
        ErrorCategory::Edit => "edit",
        ErrorCategory::Media => "media",
        ErrorCategory::Bounds => "bounds",
        ErrorCategory::Parse => "parse",
        ErrorCategory::Validation => "validation",
        ErrorCategory::Patch => "patch",
        ErrorCategory::Selector => "selector",
        ErrorCategory::Permission => "permission",
        ErrorCategory::Write => "write",
        ErrorCategory::Internal => "internal",
    }
}

#[cfg(test)]
#[test]
fn error_maps_to_canonical_envelope() {
    use pptx_compose::core::error::ErrorCode;

    let error = CoreError::new(
        ErrorCode::StalePatch,
        "Patch base_revision does not match current revision.",
    );

    let tool_error = map_error(error);
    let envelope = tool_error
        .structured_content
        .expect("tool error has structured content");

    assert_eq!(tool_error.is_error, Some(true));
    assert_eq!(envelope["error"]["code"], "stale_patch");
}

#[cfg(test)]
#[test]
fn summary_output_has_schema() {
    let schema = schemars::schema_for!(DocumentSummaryOutput);
    let value = serde_json::to_value(schema).expect("schema serializes");

    assert!(value.as_object().is_some_and(|object| !object.is_empty()));
}

#[cfg(test)]
#[test]
fn error_envelope_uses_core_code_string() {
    use pptx_compose::core::error::ErrorCode;

    let error = CoreError::new(ErrorCode::MalformedXml, "Malformed XML.");
    let envelope = error_envelope(&error);

    assert_eq!(envelope.error.code, "malformed_xml");
}

#[cfg(test)]
#[test]
fn summary_stub_uses_result_envelope_schema() {
    let output = DocumentSummaryOutput::stub("pptx_get_document_summary");

    assert_eq!(output.0.schema, "pptx-compose.result.v1");
    assert_eq!(output.0.version, 1);
}
