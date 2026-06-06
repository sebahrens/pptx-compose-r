mod legacy_compat {
    use std::{
        collections::BTreeMap,
        fs,
        io::Read,
        path::{Path, PathBuf},
        process::Command,
    };

    use zip::ZipArchive;

    #[test]
    fn commands_use_legacy_schema() {
        let temp_dir = fresh_temp_dir("commands_use_legacy_schema");
        let fixture = repo_root().join("fixtures/legacy/sample.pptx");
        let json_output = temp_dir.join("out.json");
        let convert_output = temp_dir.join("convert.json");
        let roundtrip = temp_dir.join("roundtrip.pptx");

        run_cli([
            "to-json",
            fixture.to_str().expect("fixture path is UTF-8"),
            json_output.to_str().expect("json path is UTF-8"),
            "--compat-json",
        ]);
        run_cli([
            "convert",
            fixture.to_str().expect("fixture path is UTF-8"),
            convert_output.to_str().expect("convert path is UTF-8"),
            "--compat-json",
        ]);

        let legacy = read_json(&json_output);
        let converted = read_json(&convert_output);
        assert_eq!(legacy, converted);
        assert!(legacy.get("ppt/presentation.xml").is_some_and(has_xml));
        assert!(
            legacy
                .as_object()
                .expect("legacy JSON is an object")
                .iter()
                .any(|(path, value)| path.starts_with("ppt/media/") && has_binary(value))
        );

        run_cli([
            "to-pptx",
            json_output.to_str().expect("json path is UTF-8"),
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--compat-json",
        ]);

        assert_eq!(media_bytes(&fixture), media_bytes(&roundtrip));
        assert_eq!(
            zip_entry_bytes(&fixture, "[Content_Types].xml"),
            zip_entry_bytes(&roundtrip, "[Content_Types].xml")
        );

        let validation_output = temp_dir.join("validation.json");
        run_cli([
            "validate",
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--report",
            validation_output
                .to_str()
                .expect("validation path is UTF-8"),
        ]);
        let validation = read_json(&validation_output);
        assert_eq!(
            validation.get("status").and_then(serde_json::Value::as_str),
            Some("valid")
        );
    }

    #[test]
    fn powerpoint_media_chart_embedded_legacy_roundtrip_preserves_content_types() {
        let temp_dir = fresh_temp_dir(
            "powerpoint_media_chart_embedded_legacy_roundtrip_preserves_content_types",
        );
        let fixture = repo_root().join("fixtures/powerpoint/media-chart-embedded.pptx");
        let json_output = temp_dir.join("roundtrip.json");
        let roundtrip = temp_dir.join("roundtrip.pptx");

        run_cli([
            "to-json",
            fixture.to_str().expect("fixture path is UTF-8"),
            json_output.to_str().expect("json path is UTF-8"),
            "--compat-json",
        ]);
        run_cli([
            "to-pptx",
            json_output.to_str().expect("json path is UTF-8"),
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--compat-json",
        ]);

        assert_eq!(media_bytes(&fixture), media_bytes(&roundtrip));
        assert_eq!(
            zip_entry_bytes(&fixture, "[Content_Types].xml"),
            zip_entry_bytes(&roundtrip, "[Content_Types].xml")
        );

        let validation_output = temp_dir.join("validation.json");
        run_cli([
            "validate",
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--report",
            validation_output
                .to_str()
                .expect("validation path is UTF-8"),
        ]);
        let validation = read_json(&validation_output);
        assert_eq!(
            validation.get("status").and_then(serde_json::Value::as_str),
            Some("valid")
        );
    }

    #[test]
    fn bar_chart_legacy_roundtrip_preserves_content_types() {
        let temp_dir = fresh_temp_dir("bar_chart_legacy_roundtrip_preserves_content_types");
        let fixture = repo_root().join("fixtures/charts/bar-chart.pptx");
        let json_output = temp_dir.join("roundtrip.json");
        let roundtrip = temp_dir.join("roundtrip.pptx");

        run_cli([
            "to-json",
            fixture.to_str().expect("fixture path is UTF-8"),
            json_output.to_str().expect("json path is UTF-8"),
            "--compat-json",
        ]);
        run_cli([
            "to-pptx",
            json_output.to_str().expect("json path is UTF-8"),
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--compat-json",
        ]);

        assert_eq!(
            zip_entry_bytes(&fixture, "[Content_Types].xml"),
            zip_entry_bytes(&roundtrip, "[Content_Types].xml")
        );

        let validation_output = temp_dir.join("validation.json");
        run_cli([
            "validate",
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--report",
            validation_output
                .to_str()
                .expect("validation path is UTF-8"),
        ]);
        let validation = read_json(&validation_output);
        assert_eq!(
            validation.get("status").and_then(serde_json::Value::as_str),
            Some("valid")
        );
    }

    #[test]
    fn embedded_ole_object_legacy_roundtrip_preserves_content_types() {
        let temp_dir =
            fresh_temp_dir("embedded_ole_object_legacy_roundtrip_preserves_content_types");
        let fixture = repo_root().join("fixtures/embedded/ole-object.pptx");
        let json_output = temp_dir.join("roundtrip.json");
        let roundtrip = temp_dir.join("roundtrip.pptx");

        run_cli([
            "to-json",
            fixture.to_str().expect("fixture path is UTF-8"),
            json_output.to_str().expect("json path is UTF-8"),
            "--compat-json",
        ]);
        run_cli([
            "to-pptx",
            json_output.to_str().expect("json path is UTF-8"),
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--compat-json",
        ]);

        assert_eq!(media_bytes(&fixture), media_bytes(&roundtrip));
        assert_eq!(
            zip_entry_bytes(&fixture, "[Content_Types].xml"),
            zip_entry_bytes(&roundtrip, "[Content_Types].xml")
        );

        let validation_output = temp_dir.join("validation.json");
        run_cli([
            "validate",
            roundtrip.to_str().expect("roundtrip path is UTF-8"),
            "--report",
            validation_output
                .to_str()
                .expect("validation path is UTF-8"),
        ]);
        let validation = read_json(&validation_output);
        assert_eq!(
            validation.get("status").and_then(serde_json::Value::as_str),
            Some("valid")
        );
    }

    fn run_cli<const N: usize>(args: [&str; N]) {
        let output = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
            .current_dir(repo_root())
            .args(args)
            .output()
            .expect("CLI process starts");
        assert!(
            output.status.success(),
            "CLI failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn read_json(path: &Path) -> serde_json::Value {
        let bytes = fs::read(path).expect("JSON output reads");
        serde_json::from_slice(&bytes).expect("JSON output parses")
    }

    fn has_xml(value: &serde_json::Value) -> bool {
        value
            .as_object()
            .and_then(|object| object.get("$xml"))
            .and_then(serde_json::Value::as_str)
            .is_some()
    }

    fn has_binary(value: &serde_json::Value) -> bool {
        value
            .as_object()
            .and_then(|object| object.get("$binary"))
            .and_then(serde_json::Value::as_object)
            .is_some_and(|binary| {
                binary.get("encoding").and_then(serde_json::Value::as_str) == Some("base64")
                    && binary
                        .get("data")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
            })
    }

    fn media_bytes(path: &Path) -> BTreeMap<String, Vec<u8>> {
        let file = fs::File::open(path).expect("PPTX opens");
        let mut archive = ZipArchive::new(file).expect("PPTX ZIP opens");
        let mut media = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("ZIP entry opens");
            if entry.name().starts_with("ppt/media/") && !entry.is_dir() {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).expect("media bytes read");
                media.insert(entry.name().to_owned(), bytes);
            }
        }
        media
    }

    fn zip_entry_bytes(path: &Path, entry_name: &str) -> Vec<u8> {
        let file = fs::File::open(path).expect("PPTX opens");
        let mut archive = ZipArchive::new(file).expect("PPTX ZIP opens");
        let mut entry = archive.by_name(entry_name).expect("ZIP entry opens");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("entry bytes read");
        bytes
    }

    fn fresh_temp_dir(test_name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "pptx-compose-cli-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp directory creates");
        path
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root resolves")
    }
}
