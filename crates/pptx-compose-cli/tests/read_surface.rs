#![deny(warnings)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn inspect_and_validate_emit_json_documents() {
    let fixture = repo_root().join("fixtures/legacy/sample.pptx");

    let inspect = run_cli(["inspect", fixture_str(&fixture)]);
    let inspect_json = parse_stdout(&inspect);
    assert_eq!(inspect_json["schema"], "pptx-compose.agent_view.v1");
    assert_eq!(inspect_json["version"], 1);
    assert_eq!(inspect_json["view"]["mode"], "deck_summary");

    let validate = run_cli(["validate", fixture_str(&fixture), "--report", "-"]);
    let validation_json = parse_stdout(&validate);
    assert_eq!(
        validation_json["schema"],
        "pptx-compose.validation_report.v1"
    );
    assert_eq!(validation_json["version"], 1);
    assert!(validation_json["summary"].is_object());
}

#[test]
fn inspect_rejects_view_and_report_to_stdout() {
    let fixture = repo_root().join("fixtures/minimal.pptx");
    let output = run_cli_raw([
        "--json-errors",
        "inspect",
        fixture_str(&fixture),
        "--report",
        "-",
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());

    let stderr_text = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["error"]["code"], "invalid_input");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .expect("error message is a string")
            .contains("inspect cannot write both the view and --report to stdout")
    );
}

#[test]
fn inspect_stdout_view_allows_report_file() {
    let root = unique_dir();
    let fixture = repo_root().join("fixtures/minimal.pptx");
    let report = root.join("inspect.report.json");

    let output = run_cli_owned(vec![
        "inspect".to_owned(),
        fixture_str(&fixture).to_owned(),
        "--output".to_owned(),
        "-".to_owned(),
        "--report".to_owned(),
        report.to_string_lossy().into_owned(),
    ]);

    let view = parse_stdout(&output);
    assert_eq!(view["schema"], "pptx-compose.agent_view.v1");
    assert_eq!(view["version"], 1);

    let report_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&report).expect("report reads"))
            .expect("report parses as JSON");
    assert_eq!(report_json["schema"], "pptx-compose.validation_report.v1");
    assert_eq!(report_json["version"], 1);

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn json_errors_parse_failures_emit_one_error_envelope() {
    let output = run_cli_raw(["--json-errors", "--not-a-real-flag"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr_text = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert_eq!(stderr_text.lines().count(), 1);
    assert!(!stderr_text.contains("Usage:"));

    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["error"]["code"], "invalid_input");
}

#[test]
fn inspect_full_malformed_slide_xml_preserves_core_error_code() {
    let fixture = repo_root().join("fixtures/malformed/broken-slide-xml.pptx");
    let output = run_cli_raw([
        "--json-errors",
        "inspect",
        fixture_str(&fixture),
        "--detail",
        "full",
    ]);

    assert_ne!(
        output.status.code(),
        Some(50),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());

    let stderr_text = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["status"], "error");
    assert_ne!(envelope["error"]["code"], "invalid_input");
}

#[test]
fn validate_malformed_slide_xml_emits_report_and_exits_validation_failure() {
    let fixture = repo_root().join("fixtures/malformed/broken-slide-xml.pptx");
    let output = run_cli_raw(["--json-errors", "validate", fixture_str(&fixture)]);

    assert_eq!(
        output.status.code(),
        Some(30),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report = parse_stdout(&output);
    assert_eq!(report["status"], "invalid");
    assert_eq!(report["summary"]["fatal"], 1);
    assert_eq!(report["findings"][0]["code"], "malformed_xml");
    assert_eq!(report["findings"][0]["severity"], "fatal");

    let stderr_text = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["error"]["code"], "validation_failed");
}

#[test]
fn validate_reports_no_edit_errors_without_failing() {
    let root = unique_dir();
    let deck = root.join("duplicate-slide-id.pptx");
    fs::write(&deck, duplicate_slide_id_deck()).expect("deck fixture writes");

    let output = run_cli_owned(vec![
        "validate".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--report".to_owned(),
        "-".to_owned(),
    ]);

    let report = parse_stdout(&output);
    assert_eq!(report["status"], "valid");
    assert_eq!(report["summary"]["errors"], 1);
    assert_eq!(report["findings"][0]["severity"], "error");
    assert_eq!(report["findings"][0]["blocking"], false);

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn inspect_slides_accepts_single_range_and_list_scopes() {
    let root = unique_dir();
    let deck = root.join("three-slides.pptx");
    fs::write(&deck, slide_deck(3)).expect("deck fixture writes");

    let single = parse_stdout(&run_cli_owned(vec![
        "inspect".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--slides".to_owned(),
        "2".to_owned(),
    ]));
    assert_eq!(single["view"]["mode"], "slide_detail");
    assert_slide_ids(&single, &["slide-2"]);

    let canonical_single = parse_stdout(&run_cli_owned(vec![
        "inspect".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--slides".to_owned(),
        "slide-2".to_owned(),
    ]));
    assert_eq!(canonical_single["view"]["mode"], "slide_detail");
    assert_slide_ids(&canonical_single, &["slide-2"]);
    assert_eq!(canonical_single["slides"], single["slides"]);

    let range = parse_stdout(&run_cli_owned(vec![
        "inspect".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--slides".to_owned(),
        "1-2".to_owned(),
        "--detail".to_owned(),
        "summary".to_owned(),
    ]));
    assert_eq!(range["view"]["mode"], "slide_page");
    assert_slide_ids(&range, &["slide-1", "slide-2"]);

    let list = parse_stdout(&run_cli_owned(vec![
        "inspect".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--slides".to_owned(),
        "1,3".to_owned(),
    ]));
    assert_eq!(list["view"]["mode"], "slide_page");
    assert_slide_ids(&list, &["slide-1", "slide-3"]);

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn inspect_full_whole_deck_matches_union_of_scoped_slides() {
    let root = unique_dir();
    let deck = root.join("twenty-two-slides.pptx");
    fs::write(&deck, slide_deck(22)).expect("deck fixture writes");

    let whole = parse_stdout(&run_cli_owned(vec![
        "inspect".to_owned(),
        deck.to_string_lossy().into_owned(),
        "--detail".to_owned(),
        "full".to_owned(),
    ]));
    assert_eq!(whole["view"]["mode"], "slide_page");
    assert_eq!(whole["omitted_count"], 0);

    let whole_slides = whole["slides"].as_array().expect("whole slides array");
    assert_eq!(whole_slides.len(), 22);

    for slide_number in 1..=22 {
        let scoped = parse_stdout(&run_cli_owned(vec![
            "inspect".to_owned(),
            deck.to_string_lossy().into_owned(),
            "--detail".to_owned(),
            "full".to_owned(),
            "--slides".to_owned(),
            slide_number.to_string(),
        ]));
        let scoped_slide = &scoped["slides"].as_array().expect("scoped slides array")[0];
        let whole_slide = &whole_slides[slide_number - 1];

        assert_eq!(whole_slide["id"], scoped_slide["id"]);
        assert_eq!(whole_slide["elements"], scoped_slide["elements"]);
        assert!(
            !whole_slide["elements"]
                .as_array()
                .expect("whole slide elements")
                .is_empty()
        );
    }

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn inspect_output_refuses_existing_file_without_overwrite() {
    let root = unique_dir();
    let fixture = repo_root().join("fixtures/legacy/sample.pptx");
    let output_path = root.join("deck.view.json");
    fs::write(&output_path, b"original").expect("existing output writes");

    let output = run_cli_raw_owned(vec![
        "inspect".to_owned(),
        fixture_str(&fixture).to_owned(),
        "--output".to_owned(),
        output_path.to_string_lossy().into_owned(),
    ]);

    assert!(!output.status.success());
    assert_eq!(
        fs::read(&output_path).expect("existing output reads"),
        b"original"
    );

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn schema_prints_published_schema() {
    let output = run_cli(["schema", "media-manifest-v1"]);
    let schema = parse_stdout(&output);

    assert_eq!(schema["$id"], "pptx-compose.media_manifest.v1");
    assert_eq!(schema["type"], "object");
}

#[test]
fn schema_serves_every_advertised_capability_schema() {
    let output = run_cli(["capabilities"]);
    let capabilities = parse_stdout(&output);
    let schemas = capabilities["schemas"]
        .as_array()
        .expect("capabilities schemas is an array");

    for schema in schemas {
        let name = schema["name"]
            .as_str()
            .expect("capabilities schema name is a string");
        let output = run_cli_owned(vec!["schema".to_owned(), name.to_owned()]);
        let published_schema = parse_stdout(&output);
        assert_eq!(published_schema["$id"], schema["schema"]);
        assert_eq!(published_schema["type"], "object");
    }
}

#[test]
fn media_list_and_get_extract_sanitized_package_media() {
    let root = unique_dir();
    let fixture = repo_root().join("fixtures/legacy/sample.pptx");
    let extracted = root.join("image1.png");
    let report = root.join("media-report.json");

    let list = run_cli(["media", "list", fixture_str(&fixture)]);
    let list_json = parse_stdout(&list);
    assert_eq!(list_json["schema"], "pptx-compose.result.v1");
    assert!(
        list_json["result"]["media"]
            .as_array()
            .expect("media list is an array")
            .iter()
            .any(|media| media["package_path"] == "ppt/media/image1.png")
    );

    let rejected_json_flag = run_cli_raw([
        "--json-errors",
        "media",
        "list",
        fixture_str(&fixture),
        "--json",
    ]);
    assert_eq!(
        rejected_json_flag.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&rejected_json_flag.stdout),
        String::from_utf8_lossy(&rejected_json_flag.stderr)
    );
    assert!(rejected_json_flag.stdout.is_empty());
    let stderr_text = String::from_utf8(rejected_json_flag.stderr).expect("stderr is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["error"]["code"], "invalid_input");

    run_cli_owned(vec![
        "media".to_owned(),
        "get".to_owned(),
        fixture_str(&fixture).to_owned(),
        "ppt/media/image1.png".to_owned(),
        "--output".to_owned(),
        extracted.to_string_lossy().into_owned(),
        "--report".to_owned(),
        report.to_string_lossy().into_owned(),
    ]);

    let expected = media_bytes(&fixture, "ppt/media/image1.png");
    let actual = fs::read(&extracted).expect("extracted media reads");
    assert_eq!(actual, expected);

    let report_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&report).expect("report reads"))
            .expect("report parses as JSON");
    assert_eq!(report_json["schema"], "pptx-compose.result.v1");
    assert_eq!(
        report_json["result"]["package_path"],
        "ppt/media/image1.png"
    );

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn binary_output_dash_is_rejected_without_creating_dash_file() {
    let root = unique_dir();
    let minimal = root.join("minimal.pptx");
    let sample = root.join("sample.pptx");
    let patch = root.join("noop.patch.json");
    fs::write(
        &minimal,
        fs::read(repo_root().join("fixtures/minimal.pptx")).expect("fixture reads"),
    )
    .expect("minimal fixture writes");
    fs::write(
        &sample,
        fs::read(repo_root().join("fixtures/legacy/sample.pptx")).expect("fixture reads"),
    )
    .expect("sample fixture writes");
    fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");

    let apply = run_cli_raw_owned_in_dir(
        vec![
            "--json-errors".to_owned(),
            "apply".to_owned(),
            "minimal.pptx".to_owned(),
            "noop.patch.json".to_owned(),
            "--output".to_owned(),
            "-".to_owned(),
        ],
        &root,
    );
    assert_invalid_input_error(&apply);
    assert!(
        !root.join("-").exists(),
        "apply --output - must not create a literal '-' file"
    );

    let media = run_cli_raw_owned_in_dir(
        vec![
            "--json-errors".to_owned(),
            "media".to_owned(),
            "get".to_owned(),
            "sample.pptx".to_owned(),
            "ppt/media/image1.png".to_owned(),
            "--output".to_owned(),
            "-".to_owned(),
        ],
        &root,
    );
    assert_invalid_input_error(&media);
    assert!(
        !root.join("-").exists(),
        "media get --output - must not create a literal '-' file"
    );

    fs::remove_dir_all(root).expect("test dir removes");
}

fn run_cli<const N: usize>(args: [&str; N]) -> std::process::Output {
    run_cli_owned(args.into_iter().map(str::to_owned).collect())
}

fn run_cli_raw<const N: usize>(args: [&str; N]) -> std::process::Output {
    run_cli_raw_owned(args.into_iter().map(str::to_owned).collect())
}

fn run_cli_owned(args: Vec<String>) -> std::process::Output {
    let output = run_cli_raw_owned(args);
    assert_success(&output);
    output
}

fn run_cli_raw_owned(args: Vec<String>) -> std::process::Output {
    run_cli_raw_owned_in_dir(args, &repo_root())
}

fn run_cli_raw_owned_in_dir(args: Vec<String>, current_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .current_dir(current_dir)
        .args(args)
        .output()
        .expect("CLI process starts")
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "CLI failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout parses as JSON")
}

fn assert_invalid_input_error(output: &std::process::Output) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr_text = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
    assert_eq!(stderr_text.lines().count(), 1);
    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["error"]["code"], "invalid_input");
}

fn assert_slide_ids(value: &serde_json::Value, expected: &[&str]) {
    let actual = value["slides"]
        .as_array()
        .expect("slides is an array")
        .iter()
        .map(|slide| slide["id"].as_str().expect("slide id is a string"))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn media_bytes(path: &Path, package_path: &str) -> Vec<u8> {
    let file = fs::File::open(path).expect("pptx opens");
    let mut zip = zip::ZipArchive::new(file).expect("zip opens");
    let mut entry = zip.by_name(package_path).expect("media entry exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).expect("media bytes read");
    bytes
}

fn valid_noop_patch() -> &'static [u8] {
    br#"{
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": "sha256:5aec353488af9781b58006c600039722d5f3a6bb1f0c4d8667f8f98a03e33e2e",
        "base_revision": 1,
        "client_request_id": "read-surface-noop",
        "operations": []
    }"#
}

fn fixture_str(path: &Path) -> &str {
    path.to_str().expect("fixture path is UTF-8")
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crate is under crates/pptx-compose-cli")
        .to_path_buf()
}

fn unique_dir() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pptx-compose-cli-read-surface-{}-{}",
        std::process::id(),
        unique_counter()
    ));
    fs::create_dir_all(&root).expect("test dir creates");
    root
}

fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn slide_deck(slide_count: usize) -> Vec<u8> {
    deck_with_presentation(slide_count, presentation(slide_count))
}

fn duplicate_slide_id_deck() -> Vec<u8> {
    deck_with_presentation(1, presentation_with_duplicate_slide_ids())
}

fn deck_with_presentation(slide_count: usize, presentation_xml: String) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut entries = vec![
            ("[Content_Types].xml".to_owned(), content_types(slide_count)),
            ("_rels/.rels".to_owned(), root_rels()),
            ("ppt/presentation.xml".to_owned(), presentation_xml),
            (
                "ppt/_rels/presentation.xml.rels".to_owned(),
                presentation_rels(slide_count),
            ),
        ];
        entries.extend((1..=slide_count).map(|index| {
            (
                format!("ppt/slides/slide{index}.xml"),
                text_slide(&format!("Slide {index}")),
            )
        }));
        for (name, data) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(data.as_bytes()).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

fn content_types(slide_count: usize) -> String {
    let slide_overrides = (1..=slide_count)
        .map(|index| {
            format!(
                r#"  <Override PartName="/ppt/slides/slide{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
{slide_overrides}
</Types>"#
    )
}

fn root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
        .to_owned()
}

fn presentation(slide_count: usize) -> String {
    let slide_ids = (1..=slide_count)
        .map(|index| {
            let id = 255 + index;
            format!(r#"    <p:sldId id="{id}" r:id="rId{index}"/>"#)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst>
{slide_ids}
  </p:sldIdLst>
</p:presentation>"#
    )
}

fn presentation_with_duplicate_slide_ids() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId1"/>
    <p:sldId id="256" r:id="rId1"/>
  </p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

fn presentation_rels(slide_count: usize) -> String {
    let rels = (1..=slide_count)
        .map(|index| {
            format!(
                r#"  <Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{index}.xml"/>"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
{rels}
</Relationships>"#
    )
}

fn text_slide(text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
    )
}
