#![deny(warnings)]

mod evals {
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde::Deserialize;
    use serde_json::Value;

    const CLI_CASES: [&str; 8] = [
        "replace-title",
        "find-text-selector",
        "add-text-box",
        "add-image",
        "replace-image",
        "stale-revision",
        "missing-media",
        "unsupported-chart-edit",
    ];

    #[test]
    fn cli_negative_golden_transcripts() {
        assert_golden_transcript("stale-revision");
        assert_golden_transcript("missing-media");
        assert_golden_transcript("unsupported-chart-edit");
    }

    #[test]
    fn cli_replace_title_golden_transcript() {
        assert_golden_transcript("replace-title");
    }

    #[test]
    fn cli_find_text_selector_golden_transcript() {
        let actual = assert_golden_transcript("find-text-selector");
        assert_find_text_selector_output(&actual);
    }

    #[test]
    fn cli_add_text_box_golden_transcript() {
        assert_golden_transcript("add-text-box");
    }

    #[test]
    fn cli_add_image_golden_transcript() {
        assert_golden_transcript("add-image");
    }

    #[test]
    fn cli_replace_image_golden_transcript() {
        assert_golden_transcript("replace-image");
    }

    #[test]
    fn cli_corpus_contains_required_cases() {
        let repo_root = repo_root();
        for case_name in CLI_CASES {
            let case_dir = repo_root.join("evals").join("cli").join(case_name);
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
            if case_requires_patch(case_name) {
                assert!(
                    case_dir.join("patch.json").exists(),
                    "{case_name}: missing patch.json"
                );
            }
        }
    }

    #[test]
    fn edit_goldens_do_not_assert_successful_noop() {
        let repo_root = repo_root();
        for case_name in CLI_CASES {
            let case_dir = repo_root.join("evals").join("cli").join(case_name);
            let transcript = read_transcript(&case_dir.join("expected.transcript.json"), case_name);
            if !case_requires_patch(case_name) {
                continue;
            }
            if transcript.expected_exit != 0
                || patch_operation_count(&case_dir.join("patch.json")) == 0
            {
                continue;
            }

            let report = transcript
                .stdout_json
                .as_ref()
                .or(transcript.report_json.as_ref())
                .unwrap_or_else(|| panic!("{case_name}: successful edit must emit report JSON"));
            let operation_reports = report
                .get("operation_reports")
                .and_then(Value::as_array)
                .unwrap_or_else(|| {
                    panic!("{case_name}: successful edit must include operation_reports")
                });
            let changed_parts = report
                .get("changed_parts")
                .and_then(Value::as_array)
                .unwrap_or_else(|| {
                    panic!("{case_name}: successful edit must include changed_parts")
                });
            assert!(
                !operation_reports.is_empty(),
                "{case_name}: successful edit must not assert empty operation_reports"
            );
            assert!(
                !changed_parts.is_empty(),
                "{case_name}: successful edit must not assert empty changed_parts"
            );
        }
    }

    #[derive(Debug)]
    struct EvalCase {
        expected_exit: i32,
        expected_stdout: Option<Value>,
        expected_stderr: Option<Value>,
        expected_report: Option<Value>,
        output_invariants: Option<OutputInvariants>,
        command: Vec<String>,
        input: PathBuf,
        output: PathBuf,
        report: PathBuf,
        diff: PathBuf,
        temp_dir: PathBuf,
    }

    #[derive(Debug, Deserialize)]
    struct Transcript {
        command: Vec<String>,
        expected_exit: i32,
        stdout_json: Option<Value>,
        stderr_json: Option<Value>,
        report_json: Option<Value>,
        output_invariants: Option<OutputInvariants>,
    }

    #[derive(Clone, Debug, Deserialize)]
    struct OutputInvariants {
        exists: bool,
        differs_from_input: bool,
        validates: bool,
    }

    impl EvalCase {
        fn load(repo_root: &Path, case_name: &str) -> Self {
            let case_dir = repo_root.join("evals").join("cli").join(case_name);
            let transcript_path = case_dir.join("expected.transcript.json");
            let transcript = read_transcript(&transcript_path, case_name);
            let temp_dir = unique_temp_dir(case_name);
            fs::create_dir_all(&temp_dir)
                .unwrap_or_else(|err| panic!("{case_name}: temp dir should create: {err}"));

            let input_ref = fs::read_to_string(case_dir.join("input-ref.txt"))
                .unwrap_or_else(|err| panic!("{case_name}: input ref should read: {err}"));
            let input = repo_root.join(input_ref.trim());
            let patch = case_dir.join("patch.json");
            let media_manifest = case_dir.join("media-manifest.json");
            let output = temp_dir.join("output.pptx");
            let report = temp_dir.join("report.json");
            let diff = temp_dir.join("diff.json");
            assert!(input.exists(), "{case_name}: input fixture exists");
            if case_requires_patch(case_name) {
                assert!(patch.exists(), "{case_name}: patch exists");
            }

            let command = transcript
                .command
                .into_iter()
                .map(|arg| match arg.as_str() {
                    "{input}" => input.display().to_string(),
                    "{patch}" => patch.display().to_string(),
                    "{media_manifest}" => media_manifest.display().to_string(),
                    "{output}" => output.display().to_string(),
                    "{report}" => report.display().to_string(),
                    "{diff}" => diff.display().to_string(),
                    _ => arg,
                })
                .collect();

            Self {
                expected_exit: transcript.expected_exit,
                expected_stdout: transcript.stdout_json,
                expected_stderr: transcript.stderr_json,
                expected_report: transcript.report_json,
                output_invariants: transcript.output_invariants,
                command,
                input,
                output,
                report,
                diff,
                temp_dir,
            }
        }

        fn command_args(&self) -> &[String] {
            &self.command
        }
    }

    impl Drop for EvalCase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.temp_dir);
        }
    }

    fn assert_golden_transcript(case_name: &str) -> Value {
        let repo_root = repo_root();
        let bin = env!("CARGO_BIN_EXE_pptx-compose");
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

        assert_optional_json_file(
            case_name,
            "report",
            &case.report,
            case.expected_report.as_ref(),
        );
        assert_output_invariants(case_name, &case);
        case.expected_stdout
            .clone()
            .or(case.expected_report.clone())
            .or(case.expected_stderr.clone())
            .unwrap_or(Value::Null)
    }

    fn assert_find_text_selector_output(output: &Value) {
        assert_schema_validates("find-text-selector", output);
        let matches = output["matches"]
            .as_array()
            .expect("find-text-selector: matches is an array");
        let first = matches
            .first()
            .expect("find-text-selector: at least one match is returned");
        let selector = &first["selector"];
        assert_eq!(selector["type"], "element_id");
        assert_eq!(selector["id"], first["element_id"]);

        let guards = selector["guards"]
            .as_object()
            .expect("find-text-selector: selector guards is an object");
        let actual_keys = guards.keys().cloned().collect::<BTreeSet<_>>();
        let expected_keys = ["fingerprint", "kind", "part", "slide_id", "text_hash"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_keys, expected_keys);
        assert_eq!(guards["slide_id"], first["slide_id"]);
        assert_eq!(guards["kind"], first["kind"]);
        assert_eq!(guards["part"], first["part"]);
        assert_eq!(guards["text_hash"], first["text_hash"]);
        assert_eq!(guards["fingerprint"], first["fingerprint"]);
    }

    fn assert_schema_validates(case_name: &str, actual: &Value) {
        let schema = pptx_compose::json::schemas::find_text_json_schema()
            .unwrap_or_else(|err| panic!("{case_name}: find-text schema should generate: {err:?}"));
        let validator = jsonschema::validator_for(&schema)
            .unwrap_or_else(|err| panic!("{case_name}: find-text schema compiles: {err}"));
        assert!(
            validator.is_valid(actual),
            "{case_name}: find-text output should validate against schema"
        );
    }

    fn read_transcript(transcript_path: &Path, case_name: &str) -> Transcript {
        serde_json::from_slice(
            &fs::read(transcript_path)
                .unwrap_or_else(|err| panic!("{case_name}: transcript should read: {err}")),
        )
        .unwrap_or_else(|err| panic!("{case_name}: transcript should parse: {err}"))
    }

    fn patch_operation_count(patch_path: &Path) -> usize {
        let patch: Value =
            serde_json::from_slice(&fs::read(patch_path).unwrap_or_else(|err| {
                panic!("patch should read at {}: {err}", patch_path.display())
            }))
            .unwrap_or_else(|err| panic!("patch should parse at {}: {err}", patch_path.display()));
        patch
            .get("operations")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    }

    fn case_requires_patch(case_name: &str) -> bool {
        !matches!(case_name, "find-text-selector")
    }

    fn assert_optional_json_file(
        case_name: &str,
        file_name: &str,
        path: &Path,
        expected: Option<&Value>,
    ) {
        match expected {
            Some(expected) => {
                let actual: Value =
                    serde_json::from_slice(&fs::read(path).unwrap_or_else(|err| {
                        panic!("{case_name}: {file_name} should read: {err}")
                    }))
                    .unwrap_or_else(|err| panic!("{case_name}: {file_name} parses: {err}"));
                assert_eq!(&actual, expected, "{case_name}: {file_name} JSON mismatch");
            }
            None => assert!(
                !path.exists(),
                "{case_name}: unexpected {file_name} file at {}",
                path.display()
            ),
        }
    }

    fn assert_output_invariants(case_name: &str, case: &EvalCase) {
        let Some(invariants) = &case.output_invariants else {
            assert!(
                !case.output.exists(),
                "{case_name}: unexpected output file at {}",
                case.output.display()
            );
            assert!(
                !case.diff.exists(),
                "{case_name}: unexpected diff file at {}",
                case.diff.display()
            );
            return;
        };

        assert_eq!(
            case.output.exists(),
            invariants.exists,
            "{case_name}: output existence mismatch"
        );
        if invariants.differs_from_input {
            let input = fs::read(&case.input)
                .unwrap_or_else(|err| panic!("{case_name}: input should read: {err}"));
            let output = fs::read(&case.output)
                .unwrap_or_else(|err| panic!("{case_name}: output should read: {err}"));
            assert_ne!(
                output, input,
                "{case_name}: output should mutate input bytes"
            );
        }
        if invariants.validates {
            let document = pptx_compose::PresentationDocument::open_path(&case.output)
                .unwrap_or_else(|err| panic!("{case_name}: output should open: {err}"));
            let report = document
                .validate()
                .unwrap_or_else(|err| panic!("{case_name}: output should validate: {err}"));
            assert_eq!(
                report.status,
                pptx_compose::json::schemas::ValidationStatus::Valid,
                "{case_name}: output is valid"
            );
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

    fn unique_temp_dir(case_name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after unix epoch")
            .as_nanos();
        path.push(format!(
            "pptx-compose-cli-eval-{case_name}-{}-{nanos}",
            std::process::id()
        ));
        path
    }
}
