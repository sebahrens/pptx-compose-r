#![deny(warnings)]

use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    process::Command,
};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[test]
fn global_compressed_limit_changes_inspect_open_limit() {
    let root = unique_dir();
    let input = root.join("input.pptx");
    let package = minimal_package::<0>([]);
    fs::write(&input, &package).expect("package writes");
    let max_compressed = package
        .len()
        .checked_sub(1)
        .expect("test package is non-empty")
        .to_string();

    let output_result = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args([
            "--json-errors",
            "--max-compressed-bytes",
            &max_compressed,
            "inspect",
        ])
        .arg(&input)
        .args(["--format", "agent-json"])
        .output()
        .expect("pptx-compose should run");

    assert_resource_limit_exit(&output_result);
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn global_uncompressed_limit_changes_open_limit() {
    let root = unique_dir();
    let input = root.join("input.pptx");
    fs::write(&input, minimal_package::<0>([])).expect("package writes");

    let output_result = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args(["--json-errors", "--max-uncompressed-bytes", "10", "inspect"])
        .arg(&input)
        .args(["--format", "agent-json"])
        .output()
        .expect("pptx-compose should run");

    assert_resource_limit_exit(&output_result);
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn global_part_count_limit_changes_open_limit() {
    let root = unique_dir();
    let input = root.join("input.pptx");
    fs::write(
        &input,
        minimal_package([("docProps/core.xml", b"<cp/>".as_slice())]),
    )
    .expect("package writes");

    let output_result = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args(["--json-errors", "--max-part-count", "1", "inspect"])
        .arg(&input)
        .args(["--format", "agent-json"])
        .output()
        .expect("pptx-compose should run");

    assert_resource_limit_exit(&output_result);
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn global_media_limit_changes_open_limit() {
    let root = unique_dir();
    let input = root.join("input.pptx");
    fs::write(
        &input,
        minimal_package([("ppt/media/image1.png", b"abcdef".as_slice())]),
    )
    .expect("package writes");

    let output_result = Command::new(env!("CARGO_BIN_EXE_pptx-compose"))
        .args(["--json-errors", "--max-media-bytes", "5", "inspect"])
        .arg(&input)
        .args(["--format", "agent-json"])
        .output()
        .expect("pptx-compose should run");

    assert_resource_limit_exit(&output_result);
    fs::remove_dir_all(root).expect("test dir removes");
}

fn assert_resource_limit_exit(output: &std::process::Output) {
    assert_eq!(
        output.status.code(),
        Some(12),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#""code":"resource_limit_exceeded""#),
        "{stderr}"
    );
}

fn minimal_package<const N: usize>(entries: [(&str, &[u8]); N]) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer
            .start_file("[Content_Types].xml", options)
            .expect("start content types");
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/></Types>"#,
            )
            .expect("write content types");
        for (name, contents) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(contents).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP package");
    }
    bytes.into_inner()
}

fn unique_dir() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pptx-compose-cli-resource-limits-{}-{}",
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
