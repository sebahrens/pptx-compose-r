#![deny(warnings)]

use std::{fs, path::Path};

const WORKFLOWS: [&str; 2] = [
    ".github/workflows/pull-request.yml",
    ".github/workflows/release.yml",
];

const REQUIRED_COMMANDS: [&str; 13] = [
    "cargo fmt --all --check",
    "cargo build --workspace",
    "cargo test --workspace",
    "cargo clippy --workspace --all-targets -- -D warnings",
    "cargo test -p pptx-compose-cli --test read_surface schema_prints_published_schema -- --exact",
    "cargo test -p pptx-compose-core --test fixture_manifest",
    "cargo test -p pptx-compose-core --test roundtrip",
    "cargo test -p pptx-compose --test roundtrip_golden",
    "cargo test -p pptx-compose-edit --test construction_golden",
    "cargo test -p pptx-compose-cli --test eval_transcripts",
    "cargo test -p pptx-compose-mcp --test resources_prompts",
    "cargo test -p pptx-compose-mcp --test stdio_binary",
    "Skipping",
];

#[test]
fn ci_workflows_expose_readiness_gate_commands() {
    let root = workspace_root();

    for workflow in WORKFLOWS {
        let path = root.join(workflow);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));

        assert!(
            text.contains("Local equivalents"),
            "{workflow} must document local equivalent commands"
        );
        for command in REQUIRED_COMMANDS {
            assert!(
                text.contains(command),
                "{workflow} is missing CI readiness gate command: {command}"
            );
        }
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("core crate is under crates/pptx-compose-core")
}
