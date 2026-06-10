use std::{
    borrow::Cow,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pptx_compose::core::error::{Error as CoreError, ErrorCategory, ErrorSeverity};
use pptx_compose::json::{
    schema_versions::{ERROR_SCHEMA, ERROR_VERSION, RESULT_SCHEMA, RESULT_VERSION},
    schemas::{PatchReport, ResultEnvelope, ResultStatus},
};
use rmcp::model::CallToolResult;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use serde_json::{Value, json};

macro_rules! result_output {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub ResultEnvelope);

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn schema_id() -> Cow<'static, str> {
                concat!(module_path!(), "::", stringify!($name)).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                result_or_error_schema(generator)
            }
        }
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

macro_rules! impl_success_value {
    ($($name:ident),+ $(,)?) => {
        $(
            impl $name {
                #[must_use]
                pub fn success(result: Value) -> Self {
                    Self(success_envelope(result))
                }
            }
        )+
    };
}

impl_success_value!(
    DocumentSummaryOutput,
    ListSlidesOutput,
    SlideOutput,
    ListElementsOutput,
    ElementOutput,
    ValidatePatchOutput,
    ValidateOutput,
    ExportOutput,
);

impl ApplyPatchOutput {
    #[must_use]
    pub fn applied(
        session_id: &str,
        revision: u64,
        dry_run: bool,
        client_request_id: &str,
        transaction_id: &str,
        mut report: PatchReport,
    ) -> Self {
        report.client_request_id = Some(client_request_id.to_owned());
        report.request_id = Some(client_request_id.to_owned());
        report.transaction_id = Some(transaction_id.to_owned());
        let changed_parts = report.changed_parts.clone();
        Self(success_envelope(json!({
            "session_id": session_id,
            "revision": revision,
            "dry_run": dry_run,
            "client_request_id": client_request_id,
            "request_id": client_request_id,
            "transaction_id": transaction_id,
            "changed_parts": changed_parts,
            "report": report
        })))
    }
}

impl ExportOutput {
    #[must_use]
    pub fn exported(
        session_id: String,
        client_request_id: Option<String>,
        transaction_id: String,
        changed_parts: Vec<String>,
        details: Value,
    ) -> Self {
        let mut result = json!({
            "session_id": session_id,
            "client_request_id": client_request_id,
            "request_id": client_request_id,
            "transaction_id": transaction_id,
            "changed_parts": changed_parts,
        });
        if let (Some(result), Some(details)) = (result.as_object_mut(), details.as_object()) {
            result.extend(details.clone());
        }
        Self(success_envelope(result))
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

#[cfg(test)]
mod tests {
    use pptx_compose::core::error::{Error as CoreError, ErrorCode};

    #[test]
    fn mcp_error_envelope_preserves_state_changed_marker() {
        let error = CoreError::new(
            ErrorCode::WriteFailed,
            "Export failed after writing the output file.",
        )
        .with_state_changed(true);

        let envelope = super::error_envelope(&error);

        assert!(envelope.error.state_changed);
        assert_eq!(envelope.error.code, ErrorCode::WriteFailed.as_str());
    }
}

fn result_or_error_schema(generator: &mut SchemaGenerator) -> Schema {
    json!({
        "type": "object",
        "anyOf": [
            generator.subschema_for::<ResultEnvelope>().to_value(),
            generator.subschema_for::<ErrorEnvelope>().to_value()
        ]
    })
    .try_into()
    .expect("object schema is valid")
}

#[must_use]
pub fn transaction_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("txn_{nanos:x}_{counter:x}")
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
