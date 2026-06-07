#![deny(warnings)]

use std::process::Command;

#[test]
fn help_and_version_exit_successfully() {
    let bin = env!("CARGO_BIN_EXE_pptx-compose");

    for args in [
        ["--version"].as_slice(),
        ["--help"].as_slice(),
        ["inspect", "--help"].as_slice(),
    ] {
        let output = Command::new(bin)
            .args(args)
            .output()
            .unwrap_or_else(|err| panic!("pptx-compose {args:?} should run: {err}"));

        assert_eq!(
            output.status.code(),
            Some(0),
            "pptx-compose {args:?} should exit 0\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn inspect_help_documents_slide_scope_forms() {
    let output = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args(["inspect", "--help"])
        .output()
        .expect("inspect help runs");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("slide-N"), "{stdout}");
    assert!(stdout.contains("N-M"), "{stdout}");
}

#[test]
fn spec_071_documents_global_help_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .arg("--help")
        .output()
        .expect("top-level help runs");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    let spec = include_str!("../../../specs/071-cli-agent-contract.md");

    for flag in [
        "--json-errors",
        "--quiet",
        "--verbose",
        "--no-color",
        "--workspace",
        "--temp-dir",
        "--max-compressed-bytes",
        "--max-uncompressed-bytes",
        "--max-part-count",
        "--max-media-bytes",
        "--help",
        "--version",
    ] {
        assert!(stdout.contains(flag), "top-level help must expose {flag}");
        assert!(spec.contains(flag), "spec 071 must document {flag}");
    }
    assert!(
        !stdout.contains("--keep-temp"),
        "debug temp retention flag must not be agent-facing help"
    );
    assert!(
        !spec.contains("--keep-temp"),
        "debug temp retention flag must not be in spec 071"
    );
}

#[test]
fn spec_071_documents_apply_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args(["apply", "--help"])
        .output()
        .expect("apply help runs");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    let spec = include_str!("../../../specs/071-cli-agent-contract.md");

    for flag in [
        "--dry-run",
        "--media-manifest",
        "--media-root",
        "--media",
        "--output",
        "--report",
        "--diff",
        "--overwrite",
        "--in-place",
        "--no-backup",
    ] {
        assert!(stdout.contains(flag), "apply help must expose {flag}");
        assert!(spec.contains(flag), "spec 071 must document {flag}");
    }
}
