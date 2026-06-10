#![deny(warnings)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use pptx_compose::edit::patch::{PATCH_SCHEMA, PATCH_VERSION};
use pptx_compose_mcp::{
    ApplyPatchInput, CloseInput, ExportInput, FindTextInput, GetElementInput, GetSlideInput,
    ImportMediaInput, ListElementsInput, ListSlidesInput, OpenInput, PptxServer, SummaryInput,
    ValidateInput, ValidatePatchInput, permissions::PermissionPolicy,
};
use rmcp::handler::server::wrapper::Parameters;
use schemars::JsonSchema;
use serde_json::Value;
use serde_json::json;

const MCP_CASES: [&str; 5] = [
    "inspect-large-deck",
    "patch-after-pagination",
    "media-import-add-image",
    "stale-revision",
    "validation-error-explain",
];

#[tokio::test]
async fn mcp_eval_corpus_contains_required_cases() {
    let repo_root = repo_root();
    for case_name in MCP_CASES {
        let case_dir = repo_root.join("evals").join("mcp").join(case_name);
        assert!(case_dir.is_dir(), "{case_name}: case directory exists");
        for file_name in [
            "input-ref.txt",
            "instruction.txt",
            "expected.transcript.json",
        ] {
            assert!(
                case_dir.join(file_name).exists(),
                "{case_name}: missing {file_name}"
            );
        }

        let input_ref = fs::read_to_string(case_dir.join("input-ref.txt"))
            .unwrap_or_else(|err| panic!("{case_name}: input ref reads: {err}"));
        let input = repo_root.join(input_ref.trim());
        assert!(input.exists(), "{case_name}: input fixture exists");

        let transcript = read_json(&case_dir.join("expected.transcript.json"), case_name);
        assert_eq!(
            transcript["schema"], "pptx-compose.mcp_eval_transcript.v1",
            "{case_name}: transcript schema"
        );
        assert_eq!(transcript["version"], 1, "{case_name}: transcript version");
        assert_eq!(
            transcript["case"], case_name,
            "{case_name}: transcript case"
        );
        assert_eq!(
            transcript["input_fixture"],
            input_ref.trim(),
            "{case_name}: transcript input_fixture matches input-ref.txt"
        );
        let instruction = fs::read_to_string(case_dir.join("instruction.txt"))
            .unwrap_or_else(|err| panic!("{case_name}: instruction reads: {err}"));
        assert_eq!(
            transcript["instruction"].as_str().map(str::trim),
            Some(instruction.trim()),
            "{case_name}: transcript instruction matches instruction.txt"
        );
        assert!(
            transcript["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty()),
            "{case_name}: transcript has tool steps"
        );
        assert!(
            transcript["output_invariants"]
                .as_object()
                .is_some_and(|invariants| !invariants.is_empty()),
            "{case_name}: transcript declares output invariants"
        );
        assert_transcript_arguments_validate(case_name, &transcript, input_ref.trim(), &repo_root);
        replay_transcript(
            case_name,
            &case_dir,
            &transcript,
            input_ref.trim(),
            &repo_root,
        )
        .await;
        if case_name == "stale-revision" {
            assert_stale_revision_transcript(&transcript);
        }
    }
}

fn assert_transcript_arguments_validate(
    case_name: &str,
    transcript: &Value,
    input_ref: &str,
    repo_root: &Path,
) {
    let tools = transcript["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{case_name}: tools is an array"));
    for (index, step) in tools.iter().enumerate() {
        let name = step["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{case_name}: tool step {index} has a string name"));
        let arguments = substitute_placeholders(step["arguments"].clone(), input_ref);
        assert!(
            step["expect"]["status"].is_string(),
            "{case_name}: tool step {index} declares expected status"
        );
        assert_tool_arguments_validate(case_name, index, name, &arguments);
        assert_tool_runtime_argument_invariants(case_name, index, name, &arguments, repo_root);
    }
}

fn assert_tool_arguments_validate(case_name: &str, index: usize, name: &str, arguments: &Value) {
    let schema = tool_input_schema(name)
        .unwrap_or_else(|| panic!("{case_name}: tool step {index} uses unknown tool {name}"));
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|err| panic!("{case_name}: tool {name} input schema compiles: {err}"));
    assert!(
        validator.is_valid(arguments),
        "{case_name}: tool step {index} {name} arguments validate against input schema; arguments={arguments}"
    );
}

fn assert_tool_runtime_argument_invariants(
    case_name: &str,
    index: usize,
    name: &str,
    arguments: &Value,
    repo_root: &Path,
) {
    match name {
        "pptx_open" => {
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{case_name}: tool step {index} pptx_open has a path"));
            let resolved = repo_root.join(path);
            assert!(
                resolved.exists(),
                "{case_name}: tool step {index} pptx_open path exists after placeholder substitution: {path}"
            );
        }
        "pptx_export" => {
            let output_path = arguments.get("output_path").and_then(Value::as_str);
            let inline = arguments
                .get("inline")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            assert!(
                output_path.is_some() || inline,
                "{case_name}: tool step {index} pptx_export must set inline=true when output_path is absent"
            );
        }
        _ => {}
    }
}

fn tool_input_schema(name: &str) -> Option<Value> {
    let schemas: BTreeMap<&'static str, Value> = BTreeMap::from([
        ("pptx_open", schema_for::<OpenInput>()),
        ("pptx_get_document_summary", schema_for::<SummaryInput>()),
        ("pptx_list_slides", schema_for::<ListSlidesInput>()),
        ("pptx_get_slide", schema_for::<GetSlideInput>()),
        ("pptx_list_elements", schema_for::<ListElementsInput>()),
        ("pptx_get_element", schema_for::<GetElementInput>()),
        ("pptx_find_text", schema_for::<FindTextInput>()),
        ("pptx_import_media", schema_for::<ImportMediaInput>()),
        ("pptx_validate_patch", schema_for::<ValidatePatchInput>()),
        ("pptx_apply_patch", schema_for::<ApplyPatchInput>()),
        ("pptx_validate", schema_for::<ValidateInput>()),
        ("pptx_export", schema_for::<ExportInput>()),
        ("pptx_close", schema_for::<CloseInput>()),
    ]);
    schemas.get(name).cloned()
}

fn schema_for<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("input schema serializes")
}

fn substitute_placeholders(value: Value, input_ref: &str) -> Value {
    match value {
        Value::String(value) => substitute_string_placeholder(&value, input_ref),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| substitute_placeholders(value, input_ref))
                .collect::<Vec<_>>(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, substitute_placeholders(value, input_ref)))
                .collect(),
        ),
        value => value,
    }
}

fn substitute_string_placeholder(value: &str, input_ref: &str) -> Value {
    match value {
        "{input}" => json!(input_ref),
        "{session_id}" => json!("session-1"),
        "{revision}" => json!(1),
        "{new_revision}" => json!(2),
        "{expected_patch}" => expected_patch_placeholder(),
        value => Value::String(value.to_owned()),
    }
}

fn expected_patch_placeholder() -> Value {
    json!({
        "schema": PATCH_SCHEMA,
        "version": PATCH_VERSION,
        "document_id": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "base_revision": 1,
        "client_request_id": "eval-request-1",
        "operations": [
            {
                "operation_id": "op-1",
                "op": "replace_text",
                "element_id": "slide-1:shape-1",
                "text": "Updated title"
            }
        ]
    })
}

#[derive(Debug)]
struct ReplayState {
    session_id: Option<String>,
    document_id: Option<String>,
    current_revision: Option<u64>,
    media_ref: Option<String>,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            session_id: None,
            document_id: None,
            current_revision: None,
            media_ref: None,
        }
    }

    fn substitute(&self, value: Value, input_ref: &str, expected_patch: Option<&Value>) -> Value {
        match value {
            Value::String(value) => self.substitute_string(&value, input_ref, expected_patch),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| self.substitute(value, input_ref, expected_patch))
                    .collect::<Vec<_>>(),
            ),
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, self.substitute(value, input_ref, expected_patch)))
                    .collect(),
            ),
            value => value,
        }
    }

    fn substitute_string(
        &self,
        value: &str,
        input_ref: &str,
        expected_patch: Option<&Value>,
    ) -> Value {
        match value {
            "{input}" => json!(input_ref),
            "{session_id}" => json!(self.session_id()),
            "{document_id}" => json!(self.document_id()),
            "{revision}" => json!(self.current_revision()),
            "{new_revision}" => json!(self.current_revision()),
            "{stale_revision}" => json!(self.current_revision().saturating_sub(1)),
            "{media_ref}" => json!(self.media_ref()),
            "{expected_patch}" => expected_patch
                .map(|patch| self.substitute(patch.clone(), input_ref, None))
                .unwrap_or_else(|| panic!("transcript references expected_patch but none exists")),
            value => Value::String(value.to_owned()),
        }
    }

    fn session_id(&self) -> &str {
        self.session_id
            .as_deref()
            .expect("session_id placeholder used after pptx_open")
    }

    fn document_id(&self) -> &str {
        self.document_id
            .as_deref()
            .expect("document_id placeholder used after pptx_open")
    }

    fn current_revision(&self) -> u64 {
        self.current_revision
            .expect("revision placeholder used after pptx_open")
    }

    fn media_ref(&self) -> &str {
        self.media_ref
            .as_deref()
            .expect("media_ref placeholder used after pptx_import_media")
    }
}

async fn replay_transcript(
    case_name: &str,
    case_dir: &Path,
    transcript: &Value,
    input_ref: &str,
    repo_root: &Path,
) {
    let server = PptxServer::with_permissions(PermissionPolicy::new(
        repo_root.to_path_buf(),
        std::env::temp_dir(),
        false,
    ));
    let expected_patch = case_dir
        .join("expected.patch.json")
        .exists()
        .then(|| read_json(&case_dir.join("expected.patch.json"), case_name));
    let mut state = ReplayState::new();
    let tools = transcript["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{case_name}: tools is an array"));

    for (index, step) in tools.iter().enumerate() {
        let name = step["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{case_name}: tool step {index} has a string name"));
        let arguments = state.substitute(
            step["arguments"].clone(),
            input_ref,
            expected_patch.as_ref(),
        );
        let revision_before = state.current_revision;
        let actual = call_tool(&server, name, arguments, case_name, index).await;
        assert_tool_expectation(case_name, index, name, step, &actual, revision_before);
        state.update(name, &actual);
    }
}

async fn call_tool(
    server: &PptxServer,
    name: &str,
    arguments: Value,
    case_name: &str,
    index: usize,
) -> Value {
    match name {
        "pptx_open" => call_success(
            server
                .pptx_open(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_get_document_summary" => call_success(
            server
                .pptx_get_document_summary(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_list_slides" => call_success(
            server
                .pptx_list_slides(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_get_slide" => call_success(
            server
                .pptx_get_slide(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_list_elements" => call_success(
            server
                .pptx_list_elements(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_get_element" => call_success(
            server
                .pptx_get_element(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_find_text" => call_success(
            server
                .pptx_find_text(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_import_media" => call_success(
            server
                .pptx_import_media(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_validate_patch" => call_success(
            server
                .pptx_validate_patch(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_apply_patch" => call_success(
            server
                .pptx_apply_patch(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_validate" => call_success(
            server
                .pptx_validate(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_export" => call_success(
            server
                .pptx_export(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        "pptx_close" => call_success(
            server
                .pptx_close(Parameters(parse_arguments(
                    arguments, case_name, index, name,
                )))
                .await,
        ),
        _ => panic!("{case_name}: tool step {index} uses unknown tool {name}"),
    }
}

fn parse_arguments<T: serde::de::DeserializeOwned>(
    arguments: Value,
    case_name: &str,
    index: usize,
    name: &str,
) -> T {
    serde_json::from_value(arguments).unwrap_or_else(|err| {
        panic!("{case_name}: tool step {index} {name} arguments parse: {err}")
    })
}

fn call_success<T: serde::Serialize>(
    result: Result<rmcp::Json<T>, rmcp::model::CallToolResult>,
) -> Value {
    match result {
        Ok(output) => serde_json::to_value(output.0).expect("tool output serializes"),
        Err(error) => error
            .structured_content
            .unwrap_or_else(|| json!({ "status": "error" })),
    }
}

fn assert_tool_expectation(
    case_name: &str,
    index: usize,
    name: &str,
    step: &Value,
    actual: &Value,
    revision_before: Option<u64>,
) {
    let expect = &step["expect"];
    let expected_status = expect["status"]
        .as_str()
        .unwrap_or_else(|| panic!("{case_name}: tool step {index} expected status is a string"));
    assert_eq!(
        actual["status"], expected_status,
        "{case_name}: tool step {index} {name} status matches actual={actual}"
    );

    if expected_status == "error" {
        assert_error_expectation(case_name, index, name, expect, actual, revision_before);
        return;
    }

    assert_success_expectation(case_name, index, name, expect, actual, revision_before);
}

fn assert_success_expectation(
    case_name: &str,
    index: usize,
    name: &str,
    expect: &Value,
    actual: &Value,
    revision_before: Option<u64>,
) {
    if let Some(fields) = expect["result_fields"].as_array() {
        for field in fields {
            let field = field.as_str().unwrap_or_else(|| {
                panic!("{case_name}: tool step {index} result field is a string")
            });
            assert!(
                !actual["result"][field].is_null(),
                "{case_name}: tool step {index} {name} result contains {field}; actual={actual}"
            );
        }
    }
    if let Some(view_mode) = expect["view_mode"].as_str() {
        assert_eq!(
            actual["result"]["view"]["mode"], view_mode,
            "{case_name}: tool step {index} {name} view mode"
        );
    }
    if let Some(patch_status) = expect["patch_status"].as_str() {
        assert_eq!(
            actual["result"]["status"], patch_status,
            "{case_name}: tool step {index} {name} patch status"
        );
    }
    if let Some(validation_status) = expect["validation_status"].as_str() {
        assert_eq!(
            actual["result"]["status"], validation_status,
            "{case_name}: tool step {index} {name} validation status"
        );
    }
    if let Some(inline_pptx) = expect["inline_pptx"].as_bool() {
        let has_inline_pptx = actual["result"]["inline"]["data"].as_str().is_some();
        assert_eq!(
            has_inline_pptx, inline_pptx,
            "{case_name}: tool step {index} {name} inline export"
        );
    }
    if let Some(increment) = expect["revision_increment"].as_u64() {
        let before = revision_before
            .unwrap_or_else(|| panic!("{case_name}: revision increment before open"));
        assert_eq!(
            actual["result"]["revision"],
            before + increment,
            "{case_name}: tool step {index} {name} revision increment"
        );
    }
}

fn assert_error_expectation(
    case_name: &str,
    index: usize,
    name: &str,
    expect: &Value,
    actual: &Value,
    revision_before: Option<u64>,
) {
    if let Some(code) = expect["error"]["code"].as_str() {
        assert_eq!(
            actual["error"]["code"], code,
            "{case_name}: tool step {index} {name} error code"
        );
    }
    if let Some(state_changed) = expect["error"]["state_changed"].as_bool() {
        assert_eq!(
            actual["error"]["state_changed"], state_changed,
            "{case_name}: tool step {index} {name} state_changed"
        );
    }
    if let Some(increment) = expect["revision_increment"].as_u64() {
        let before = revision_before
            .unwrap_or_else(|| panic!("{case_name}: revision increment before open"));
        let current = actual["error"]["location"]["current_revision"]
            .as_u64()
            .unwrap_or(before);
        assert_eq!(
            current,
            before + increment,
            "{case_name}: tool step {index} {name} error revision increment"
        );
    }
}

impl ReplayState {
    fn update(&mut self, name: &str, actual: &Value) {
        if actual["status"] != "success" {
            return;
        }
        match name {
            "pptx_open" => {
                self.session_id = actual["result"]["session_id"]
                    .as_str()
                    .map(ToOwned::to_owned);
                self.document_id = actual["result"]["document_id"]
                    .as_str()
                    .map(ToOwned::to_owned);
                self.current_revision = actual["result"]["revision"].as_u64();
            }
            "pptx_import_media" => {
                self.media_ref = actual["result"]["media_ref"]
                    .as_str()
                    .map(ToOwned::to_owned);
            }
            "pptx_apply_patch" => {
                self.current_revision = actual["result"]["revision"].as_u64();
            }
            _ => {}
        }
    }
}

fn assert_stale_revision_transcript(transcript: &Value) {
    let tools = transcript["tools"]
        .as_array()
        .expect("stale-revision: tools is an array");
    let stale_step = tools
        .iter()
        .find(|step| {
            step["name"] == "pptx_apply_patch"
                && step["expect"]["status"] == "error"
                && step["expect"]["error"]["code"] == "stale_patch"
        })
        .expect("stale-revision: stale apply step is present");

    assert_eq!(
        stale_step["expect"]["error"]["state_changed"], false,
        "stale-revision: stale patch does not mutate state"
    );
    assert_eq!(
        stale_step["expect"]["revision_increment"], 0,
        "stale-revision: stale patch does not increment revision"
    );
    assert!(
        tools.iter().all(|step| step["name"] != "pptx_export"),
        "stale-revision: rejected stale flow does not export"
    );
}

fn read_json(path: &Path, case_name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|err| panic!("{case_name}: JSON reads: {err}")),
    )
    .unwrap_or_else(|err| panic!("{case_name}: JSON parses: {err}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is under crates/pptx-compose-mcp")
        .to_path_buf()
}
