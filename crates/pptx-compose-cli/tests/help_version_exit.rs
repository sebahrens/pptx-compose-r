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
