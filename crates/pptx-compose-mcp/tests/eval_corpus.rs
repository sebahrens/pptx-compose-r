#![deny(warnings)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use pptx_compose::edit::patch::{PATCH_SCHEMA, PATCH_VERSION};
use pptx_compose_mcp::{
    ApplyPatchInput, CloseInput, ExportInput, GetSlideInput, ImportMediaInput, ListSlidesInput,
    OpenInput, SummaryInput, ValidateInput, ValidatePatchInput,
};
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

#[test]
fn mcp_eval_corpus_contains_required_cases() {
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
