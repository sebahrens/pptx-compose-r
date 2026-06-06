#![deny(warnings)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

const MCP_CASES: [&str; 4] = [
    "inspect-large-deck",
    "patch-after-pagination",
    "media-import-add-image",
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
        assert!(
            transcript["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty()),
            "{case_name}: transcript has tool steps"
        );
    }
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
