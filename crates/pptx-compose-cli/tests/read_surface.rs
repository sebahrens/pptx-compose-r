#![deny(warnings)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn inspect_and_validate_emit_json_documents() {
    let fixture = repo_root().join("fixtures/legacy/sample.pptx");

    let inspect = run_cli(["inspect", fixture_str(&fixture), "--format", "agent-json"]);
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
fn schema_prints_published_schema() {
    let output = run_cli(["schema", "media-manifest-v1"]);
    let schema = parse_stdout(&output);

    assert_eq!(schema["$id"], "pptx-compose.media_manifest.v1");
    assert_eq!(schema["type"], "object");
}

#[test]
fn media_list_and_get_extract_sanitized_package_media() {
    let root = unique_dir();
    let fixture = repo_root().join("fixtures/legacy/sample.pptx");
    let extracted = root.join("image1.png");
    let report = root.join("media-report.json");

    let list = run_cli(["media", "list", fixture_str(&fixture), "--json"]);
    let list_json = parse_stdout(&list);
    assert_eq!(list_json["schema"], "pptx-compose.result.v1");
    assert!(
        list_json["result"]["media"]
            .as_array()
            .expect("media list is an array")
            .iter()
            .any(|media| media["package_path"] == "ppt/media/image1.png")
    );

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

fn run_cli<const N: usize>(args: [&str; N]) -> std::process::Output {
    run_cli_owned(args.into_iter().map(str::to_owned).collect())
}

fn run_cli_owned(args: Vec<String>) -> std::process::Output {
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
    output
}

fn parse_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout parses as JSON")
}

fn media_bytes(path: &Path, package_path: &str) -> Vec<u8> {
    let file = fs::File::open(path).expect("pptx opens");
    let mut zip = zip::ZipArchive::new(file).expect("zip opens");
    let mut entry = zip.by_name(package_path).expect("media entry exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).expect("media bytes read");
    bytes
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
