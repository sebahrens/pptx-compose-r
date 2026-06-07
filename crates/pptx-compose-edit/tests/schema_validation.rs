#![deny(warnings)]

use pptx_compose_edit::{
    media_inputs::media_manifest_json_schema,
    patch::{
        Operation, PATCH_SCHEMA, PATCH_VERSION, Patch, ReplaceTextOperation, patch_json_schema,
    },
};
use pptx_compose_json::{
    agent_view::{
        AgentView, Capabilities, FindTextScope, PresentationView,
        pagination::ViewMeta,
        views::{
            FindTextRequest, ViewMode, ViewRequest, build_view, find_text, package_from_pptx_bytes,
        },
    },
    schema_versions::{
        AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION, ERROR_SCHEMA, ERROR_VERSION, PATCH_REPORT_SCHEMA,
        PATCH_REPORT_VERSION, RESULT_SCHEMA, RESULT_VERSION, VALIDATION_REPORT_SCHEMA,
        VALIDATION_REPORT_VERSION,
    },
    schemas::{
        ErrorCode, ErrorEnvelope, ErrorStatus, ErrorView, OperationReport, OperationStatus,
        OperationTarget, PatchReport, PatchStatus, PatchValidationSummary, ResultEnvelope,
        ResultStatus, Severity, Summary, ValidationReport, ValidationStatus,
        agent_view_json_schema, error_json_schema, find_text_json_schema, patch_report_json_schema,
        result_json_schema, validation_report_json_schema,
    },
};
use serde_json::{Value, json};

#[test]
fn emitted_json_instances_validate_against_published_schemas() {
    assert_schema_accepts(
        agent_view_json_schema().expect("agent view schema emits"),
        serde_json::to_value(agent_view()).expect("agent view serializes"),
    );
    assert_schema_accepts(
        find_text_json_schema().expect("find-text schema emits"),
        find_text_result(),
    );
    assert_schema_accepts(
        patch_json_schema().expect("patch schema emits"),
        serde_json::to_value(patch()).expect("patch serializes"),
    );
    assert_schema_accepts(
        media_manifest_json_schema().expect("media manifest schema emits"),
        json!({
            "schema": "pptx-compose.media_manifest.v1",
            "version": 1,
            "media": {
                "hero": {
                    "path": "hero.png",
                    "content_type": "image/png",
                    "sha256": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "byte_length": 4
                }
            }
        }),
    );
    assert_schema_accepts(
        patch_report_json_schema().expect("patch report schema emits"),
        serde_json::to_value(patch_report()).expect("patch report serializes"),
    );
    assert_schema_accepts(
        validation_report_json_schema().expect("validation report schema emits"),
        serde_json::to_value(validation_report()).expect("validation report serializes"),
    );
    assert_schema_accepts(
        result_json_schema().expect("result schema emits"),
        serde_json::to_value(result_envelope()).expect("result serializes"),
    );
    assert_schema_accepts(
        error_json_schema().expect("error schema emits"),
        serde_json::to_value(error_envelope()).expect("error serializes"),
    );
}

#[test]
fn emitted_schema_rejects_out_of_schema_instance() {
    let mut instance = serde_json::to_value(result_envelope()).expect("result serializes");
    instance["unexpected"] = json!(true);

    assert_schema_rejects(result_json_schema().expect("result schema emits"), instance);
}

#[test]
fn all_agent_view_modes_validate_against_published_schema() {
    let pkg = package_from_pptx_bytes(include_bytes!(
        "../../../fixtures/real-world/worldbank-cpf-concept-note.pptx"
    ))
    .expect("fixture package parses");
    let schema = agent_view_json_schema().expect("agent view schema emits");
    let slide_detail = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
    )
    .expect("slide_detail builds");
    let element_id = slide_detail["slides"][0]["elements"][0]["id"]
        .as_str()
        .expect("fixture exposes an element")
        .to_owned();

    for request in [
        ViewRequest {
            mode: ViewMode::DeckSummary,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
        ViewRequest {
            mode: ViewMode::SlidePage,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
        ViewRequest {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
        ViewRequest {
            mode: ViewMode::ElementDetail,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: Some(element_id),
            cursor: None,
            limit: None,
        },
        ViewRequest {
            mode: ViewMode::MediaMetadata,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
        ViewRequest {
            mode: ViewMode::ValidationReport,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
    ] {
        let value = build_view(&pkg, request).expect("view mode builds");
        assert_schema_accepts(schema.clone(), value);
    }
}

fn assert_schema_accepts(schema: Value, instance: Value) {
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    assert!(
        validator.is_valid(&instance),
        "instance should validate against schema {schema}"
    );
}

fn assert_schema_rejects(schema: Value, instance: Value) {
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    assert!(
        !validator.is_valid(&instance),
        "instance should not validate against schema {schema}"
    );
}

fn agent_view() -> AgentView {
    AgentView {
        schema: AGENT_VIEW_SCHEMA.to_owned(),
        version: AGENT_VIEW_VERSION,
        document_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        revision: 1,
        view_id: "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            .to_owned(),
        view: ViewMeta {
            mode: "deck_summary".to_owned(),
            limit: 1,
            next_cursor: None,
            truncated: false,
        },
        omitted_count: 0,
        capabilities: Capabilities {
            operations: vec!["replace_text".to_owned()],
            media_content_types: vec!["image/png".to_owned()],
            units: "emu".to_owned(),
        },
        presentation: PresentationView {
            part: "ppt/presentation.xml".to_owned(),
            slide_count: 0,
        },
        slides: Vec::new(),
        warnings: Vec::new(),
        media: Vec::new(),
        validation: None,
    }
}

fn find_text_result() -> Value {
    let pkg = package_from_pptx_bytes(include_bytes!(
        "../../../fixtures/real-world/worldbank-cpf-concept-note.pptx"
    ))
    .expect("fixture package parses");
    serde_json::to_value(
        find_text(
            &pkg,
            FindTextRequest {
                query: "World Bank".to_owned(),
                scope: FindTextScope::Deck,
                cursor: None,
                limit: Some(2),
            },
        )
        .expect("find-text result builds"),
    )
    .expect("find-text result serializes")
}

fn patch() -> Patch {
    Patch {
        schema: PATCH_SCHEMA.to_owned(),
        version: PATCH_VERSION,
        document_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        base_revision: 1,
        client_request_id: "request-1".to_owned(),
        operations: vec![Operation::ReplaceText(ReplaceTextOperation {
            operation_id: "op-1".to_owned(),
            element_id: "slide-1:shape-4".to_owned(),
            selector: None,
            text: "Updated title".to_owned(),
            current_text_match: None,
            mode: None,
            format_policy: None,
            overflow_policy: None,
            allow_formatting_simplification: false,
            run_style: None,
        })],
    }
}

fn patch_report() -> PatchReport {
    PatchReport {
        schema: PATCH_REPORT_SCHEMA.to_owned(),
        version: PATCH_REPORT_VERSION,
        client_request_id: None,
        request_id: None,
        transaction_id: None,
        status: PatchStatus::Applied,
        dry_run: false,
        document_id: "sha256:old".to_owned(),
        base_revision: 1,
        new_document_id: "sha256:new".to_owned(),
        new_revision: 2,
        operation_reports: vec![OperationReport {
            operation_id: "op-1".to_owned(),
            op: "replace_text".to_owned(),
            status: OperationStatus::Applied,
            target: OperationTarget {
                slide_id: "slide-1".to_owned(),
                element_id: "slide-1:shape-4".to_owned(),
                part: "ppt/slides/slide1.xml".to_owned(),
            },
            changed_parts: vec!["ppt/slides/slide1.xml".to_owned()],
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
            error: None,
        }],
        changed_parts: vec!["ppt/slides/slide1.xml".to_owned()],
        warnings: Vec::new(),
        validation: PatchValidationSummary {
            status: ValidationStatus::Valid,
            errors: 0,
            warnings: 0,
        },
    }
}

fn validation_report() -> ValidationReport {
    ValidationReport {
        schema: VALIDATION_REPORT_SCHEMA.to_owned(),
        version: VALIDATION_REPORT_VERSION,
        document_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        revision: 1,
        status: ValidationStatus::Valid,
        summary: Summary {
            fatal: 0,
            errors: 0,
            warnings: 0,
            info: 0,
        },
        findings: Vec::new(),
    }
}

fn result_envelope() -> ResultEnvelope {
    ResultEnvelope {
        schema: RESULT_SCHEMA.to_owned(),
        version: RESULT_VERSION,
        status: ResultStatus::Success,
        result: json!({}),
        warnings: Vec::new(),
        next_cursor: None,
    }
}

fn error_envelope() -> ErrorEnvelope {
    ErrorEnvelope {
        schema: ERROR_SCHEMA.to_owned(),
        version: ERROR_VERSION,
        status: ErrorStatus::Error,
        error: ErrorView {
            code: ErrorCode::StalePatch,
            message: "Patch base_revision does not match current revision.".to_owned(),
            severity: Severity::Error,
            category: "patch".to_owned(),
            retryable: false,
            state_changed: false,
            location: json!({ "operation_id": "op-1" }),
            suggestions: vec!["Inspect the deck again and regenerate the patch.".to_owned()],
        },
    }
}
