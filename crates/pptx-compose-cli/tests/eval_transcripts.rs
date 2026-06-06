#![deny(warnings)]

mod evals {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use serde::Deserialize;
    use serde_json::Value;

    const SEED_CASES: [&str; 4] = [
        "replace-title",
        "add-image",
        "stale-revision",
        "unsupported-chart-edit",
    ];

    #[test]
    fn cli_golden_transcripts() {
        let repo_root = repo_root();
        let bin = env!("CARGO_BIN_EXE_pptx-compose");

        for case_name in SEED_CASES {
            let case = EvalCase::load(&repo_root, case_name);
            let output = Command::new(bin)
                .current_dir(&repo_root)
                .args(case.command_args())
                .output()
                .unwrap_or_else(|err| panic!("{case_name}: CLI process should run: {err}"));

            let actual_exit = output.status.code().unwrap_or(-1);
            assert_eq!(
                actual_exit,
                case.expected_exit,
                "{case_name}: exit code mismatch\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            assert_json_stream(
                case_name,
                "stdout",
                &output.stdout,
                case.expected_stdout.as_ref(),
            );
            assert_json_stream(
                case_name,
                "stderr",
                &output.stderr,
                case.expected_stderr.as_ref(),
            );
        }
    }

    #[derive(Debug)]
    struct EvalCase {
        expected_exit: i32,
        expected_stdout: Option<Value>,
        expected_stderr: Option<Value>,
        command: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct Transcript {
        command: Vec<String>,
        expected_exit: i32,
        stdout_json: Option<Value>,
        stderr_json: Option<Value>,
    }

    impl EvalCase {
        fn load(repo_root: &Path, case_name: &str) -> Self {
            let case_dir = repo_root.join("evals").join("cli").join(case_name);
            let transcript_path = case_dir.join("expected.transcript.json");
            let transcript: Transcript = serde_json::from_slice(
                &fs::read(&transcript_path)
                    .unwrap_or_else(|err| panic!("{case_name}: transcript should read: {err}")),
            )
            .unwrap_or_else(|err| panic!("{case_name}: transcript should parse: {err}"));

            let input_ref = fs::read_to_string(case_dir.join("input-ref.txt"))
                .unwrap_or_else(|err| panic!("{case_name}: input ref should read: {err}"));
            let input = repo_root.join(input_ref.trim());
            let patch = case_dir.join("patch.json");
            assert!(input.exists(), "{case_name}: input fixture exists");
            assert!(patch.exists(), "{case_name}: patch exists");

            let command = transcript
                .command
                .into_iter()
                .map(|arg| match arg.as_str() {
                    "{input}" => input.display().to_string(),
                    "{patch}" => patch.display().to_string(),
                    _ => arg,
                })
                .collect();

            Self {
                expected_exit: transcript.expected_exit,
                expected_stdout: transcript.stdout_json,
                expected_stderr: transcript.stderr_json,
                command,
            }
        }

        fn command_args(&self) -> &[String] {
            &self.command
        }
    }

    fn assert_json_stream(
        case_name: &str,
        stream_name: &str,
        bytes: &[u8],
        expected: Option<&Value>,
    ) {
        let text = String::from_utf8_lossy(bytes);
        match expected {
            Some(expected) => {
                let actual: Value = serde_json::from_str(text.trim()).unwrap_or_else(|err| {
                    panic!("{case_name}: {stream_name} should be JSON: {err}\n{text}")
                });
                assert_eq!(
                    &actual, expected,
                    "{case_name}: {stream_name} JSON mismatch"
                );
            }
            None => assert!(
                text.trim().is_empty(),
                "{case_name}: expected empty {stream_name}, got {text:?}"
            ),
        }
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crate is under crates/pptx-compose-cli")
            .to_path_buf()
    }
}
