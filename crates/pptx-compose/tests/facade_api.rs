use std::{collections::HashMap, fs, io::Cursor, io::Write, path::Path};

use pptx_compose::{
    AgentViewOptions, ApplyPatchOptions, MediaInputs, OpenOptions, Patch, PresentationDocument,
    WriteMode, WriteOptions,
    core::{
        error::ErrorCode,
        provenance::document_id::document_id as provenance_document_id,
        zip::reader::{RawEntry, from_bytes},
    },
    edit::{
        media_inputs::{MediaBinding, MediaSource},
        patch::parse_patch,
    },
    json::agent_view::{
        FindTextScope,
        views::{FindTextRequest, ViewMode},
    },
};
use serde_json::Value;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[test]
fn exposes_required_070_api_and_defaults() {
    let defaults = WriteOptions::default();
    assert_eq!(
        defaults,
        WriteOptions {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
        }
    );

    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let mut from_bytes = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let _from_reader = PresentationDocument::open_reader(Cursor::new(bytes)).expect("reader opens");

    let root = unique_dir();
    let input = root.join("input.pptx");
    let output = root.join("output.pptx");
    fs::write(&input, bytes).expect("fixture writes");
    let from_path = PresentationDocument::open_path(&input).expect("path opens");
    let _from_path_with_options =
        PresentationDocument::open_path_with_options(&input, OpenOptions::default())
            .expect("path opens with options");

    let _agent_json = from_path.to_agent_json().expect("agent view builds");
    let _agent_json_with_options = from_path
        .to_agent_json_with_options(AgentViewOptions::summary())
        .expect("agent view builds with options");
    let _legacy_json = from_path.to_legacy_json().expect("legacy JSON builds");
    let _validation = from_path.validate().expect("validation report builds");
    let _bytes = from_path.write_vec().expect("default write vec succeeds");
    let _bytes_with_options = from_path
        .write_vec_with_options(WriteOptions::default())
        .expect("write vec with options succeeds");
    from_path
        .write_path_with_options(
            &output,
            WriteOptions {
                overwrite: false,
                ..WriteOptions::default()
            },
        )
        .expect("write path with options succeeds");

    let report = from_bytes
        .apply_patch(noop_patch(bytes), MediaInputs::default())
        .expect("apply_patch returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::Applied
    );
    let report = from_bytes
        .apply_patch_with_options(
            noop_patch(bytes),
            MediaInputs::default(),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("apply_patch_with_options returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::DryRunSuccess
    );

    let output_default = root.join("default-output.pptx");
    from_bytes
        .write_path(&output_default)
        .expect("default write path succeeds");

    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn concurrent_non_overwrite_writes_do_not_clobber_output() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let document = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let root = unique_dir();
    let output = root.join("concurrent-output.pptx");
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();

    for _ in 0..2 {
        let document = document.clone();
        let output = output.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            document
                .write_path_with_options(
                    &output,
                    WriteOptions {
                        overwrite: false,
                        ..WriteOptions::default()
                    },
                )
                .map_err(|error| error.code())
        }));
    }

    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer thread finishes"))
        .collect::<Vec<_>>();
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "exactly one writer should publish the output"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ErrorCode::WriteFailed)))
            .count(),
        1,
        "the other writer should fail without replacing the output"
    );

    let output_bytes = fs::read(&output).expect("one output deck exists");
    PresentationDocument::from_bytes(&output_bytes).expect("published output reopens");
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn no_edit_rezip_preserves_canonical_document_id() {
    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let document = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let original_id = document_id(bytes);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Deterministic,
            ..WriteOptions::default()
        })
        .expect("deterministic no-edit write succeeds");
    assert_eq!(document_id(&written), original_id);

    let reopened = PresentationDocument::from_bytes(&written).expect("written deck reopens");
    let validation = reopened.validate().expect("validation report builds");
    assert_eq!(validation.document_id, original_id);
}

#[test]
fn zip_directory_entries_do_not_affect_identity_or_validation_and_are_preserved() {
    let without_dirs = text_deck();
    let with_dirs = text_deck_with_directories();
    assert_eq!(document_id(&with_dirs), document_id(&without_dirs));

    let without_dirs_doc = PresentationDocument::from_bytes(&without_dirs).expect("deck opens");
    let with_dirs_doc = PresentationDocument::from_bytes(&with_dirs).expect("deck with dirs opens");
    let without_dirs_validation = without_dirs_doc.validate().expect("deck validates");
    let with_dirs_validation = with_dirs_doc.validate().expect("deck with dirs validates");
    assert_eq!(with_dirs_validation.status, without_dirs_validation.status);
    assert_eq!(
        with_dirs_validation.document_id,
        without_dirs_validation.document_id
    );

    let written = with_dirs_doc
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("deck with dirs writes");
    let written_entries = from_bytes(&written).expect("written entries read");
    assert!(
        written_entries
            .iter()
            .any(|entry| entry.meta.is_dir && entry.meta.original_name == "ppt/")
    );
    assert!(
        written_entries
            .iter()
            .any(|entry| entry.meta.is_dir && entry.meta.original_name == "ppt/slides/")
    );
    assert_eq!(document_id(&written), document_id(&without_dirs));
}

#[test]
fn find_text_returns_selector_ready_hits() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let result = document
        .find_text(FindTextRequest {
            query: "Original".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        })
        .expect("find_text succeeds");

    assert_eq!(result.schema, "pptx-compose.find_text.v1");
    assert_eq!(result.matches.len(), 1);
    let hit = &result.matches[0];
    assert_eq!(hit.slide_id, "slide-1");
    assert_eq!(hit.matched_text, "Original");
    assert_eq!(hit.span.start, 0);
    assert_eq!(hit.span.end, 8);
    assert_eq!(hit.selector.selector_type, "element_id");
    assert_eq!(hit.selector.id, hit.element_id);
    assert_eq!(hit.selector.guards.slide_id, hit.slide_id);
    assert_eq!(hit.selector.guards.part, hit.part);
    assert_eq!(hit.selector.guards.text_hash, hit.text_hash);
    assert_eq!(hit.selector.guards.fingerprint, hit.fingerprint);
}

#[test]
fn agent_text_view_does_not_emit_unaddressable_run_or_paragraph_ids() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            slide_id: Some("slide-1".to_owned()),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("slide_detail builds");
    let text = view["slides"][0]["elements"]
        .as_array()
        .expect("slide_detail exposes elements")
        .iter()
        .find_map(|element| element.get("text"))
        .expect("text deck exposes text");

    for paragraph in text["paragraphs"].as_array().expect("paragraphs array") {
        assert_no_field(paragraph, "id");
        for run in paragraph["runs"].as_array().expect("runs array") {
            assert_no_field(run, "id");
        }
    }
}

fn assert_no_field(value: &Value, field: &str) {
    assert_eq!(value.get(field), None);
}

#[test]
fn replace_text_apply_writes_only_dirtied_slide_part() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "replace-text-facade",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": "Updated title"
        }]
    }))
    .expect("patch parses");

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("replace_text applies");
    assert_eq!(report.changed_parts, vec!["ppt/slides/slide1.xml"]);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    assert_ne!(written, bytes);

    let original_entries = from_bytes(&bytes).expect("original entries read");
    let written_entries = from_bytes(&written).expect("written entries read");
    let changed_parts = original_entries
        .iter()
        .zip(written_entries.iter())
        .filter_map(|(original, written)| {
            if original.bytes == written.bytes {
                None
            } else {
                Some(original.name.zip_entry_name().to_owned())
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(changed_parts, vec!["ppt/slides/slide1.xml"]);

    let slide = written_entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == "ppt/slides/slide1.xml")
        .expect("slide entry exists");
    let slide_xml = std::str::from_utf8(&slide.bytes).expect("slide XML is UTF-8");
    assert!(slide_xml.contains(">Updated title<"));

    let root = unique_dir();
    let output = root.join("replace-text-output.pptx");
    document
        .write_path_with_options(
            &output,
            WriteOptions {
                mode: WriteMode::Preserve,
                ..WriteOptions::default()
            },
        )
        .expect("edited deck writes to path");
    let path_written = fs::read(&output).expect("written path reads");
    fs::remove_dir_all(root).expect("test dir removes");
    assert_ne!(path_written, bytes);

    let path_written_entries = from_bytes(&path_written).expect("path-written entries read");
    let path_changed_parts = original_entries
        .iter()
        .zip(path_written_entries.iter())
        .filter_map(|(original, written)| {
            if original.bytes == written.bytes {
                None
            } else {
                Some(original.name.zip_entry_name().to_owned())
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(path_changed_parts, vec!["ppt/slides/slide1.xml"]);
}

#[test]
fn add_image_write_reopens_with_content_type_and_relationship() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "add-image-facade",
        "operations": [{
            "operation_id": "add-hero",
            "op": "add_image",
            "slide_id": "slide-1",
            "media_ref": "hero",
            "content_type": "image/png",
            "bounds": { "x": 0, "y": 0, "cx": 914400, "cy": 914400 }
        }]
    }))
    .expect("patch parses");

    let report = document
        .apply_patch(patch, media_inputs("hero", "image/png", tiny_png()))
        .expect("add_image applies");
    let mut changed_parts = report.changed_parts.clone();
    changed_parts.sort();
    assert_eq!(
        changed_parts,
        vec![
            "[Content_Types].xml",
            "ppt/media/image1.png",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/slide1.xml",
        ]
    );

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    let reopened = PresentationDocument::from_bytes(&written).expect("written deck reopens");
    let validation = reopened.validate().expect("written deck validates");
    assert_eq!(
        validation.status,
        pptx_compose::json::schemas::ValidationStatus::Valid
    );

    let entries = from_bytes(&written).expect("written entries read");
    let content_types = entry_text(&entries, "[Content_Types].xml");
    assert!(content_types.contains(r#"Extension="png""#));
    assert!(content_types.contains(r#"ContentType="image/png""#));
    let slide_rels = entry_text(&entries, "ppt/slides/_rels/slide1.xml.rels");
    assert!(slide_rels.contains(
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image""#
    ));
    assert!(slide_rels.contains(r#"Target="../media/image1.png""#));
    assert!(
        entries
            .iter()
            .any(|entry| entry.name.zip_entry_name() == "ppt/media/image1.png")
    );
}

#[test]
fn add_image_dry_run_uses_edit_layer_validation() {
    let bytes = text_deck();
    assert_add_image_dry_run_failure(
        &bytes,
        serde_json::json!({
            "bounds": { "x": 0, "y": 0, "cx": 0, "cy": 914400 }
        }),
        media_inputs("hero", "image/png", tiny_png()),
        ErrorCode::InvalidBounds,
        "invalid-bounds",
    );
    assert_add_image_dry_run_failure(
        &bytes,
        serde_json::json!({
            "fit": "contain"
        }),
        media_inputs("hero", "image/png", tiny_png()),
        ErrorCode::UnsupportedEdit,
        "unsupported-fit",
    );
    assert_add_image_dry_run_failure(
        &bytes,
        serde_json::json!({
            "dedupe": "checksum"
        }),
        media_inputs("hero", "image/png", tiny_png()),
        ErrorCode::UnsupportedEdit,
        "unsupported-dedupe",
    );
    assert_add_image_dry_run_failure(
        &bytes,
        serde_json::json!({
            "content_type": "image/webp"
        }),
        media_inputs("hero", "image/webp", b"RIFF____WEBP".to_vec()),
        ErrorCode::UnsupportedMediaType,
        "unsupported-content-type",
    );
}

#[test]
fn successful_real_applies_increment_session_revision_and_reject_stale_patch() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let initial = document.validate().expect("initial validation succeeds");
    assert_eq!(initial.revision, 1);

    let first_report = document
        .apply_patch(
            replace_title_patch(&initial.document_id, 1, "revision-first", "First update"),
            MediaInputs::default(),
        )
        .expect("first replace_text applies");
    assert_eq!(first_report.base_revision, 1);
    assert_eq!(first_report.new_revision, 2);

    let after_first = document.validate().expect("post-apply validation succeeds");
    assert_eq!(after_first.revision, 2);
    let stale_error = document
        .apply_patch(
            replace_title_patch(
                &after_first.document_id,
                1,
                "revision-stale",
                "Stale update",
            ),
            MediaInputs::default(),
        )
        .expect_err("stale base_revision is rejected");
    assert_eq!(stale_error.code(), ErrorCode::StalePatch);

    let second_report = document
        .apply_patch(
            replace_title_patch(
                &after_first.document_id,
                2,
                "revision-second",
                "Second update",
            ),
            MediaInputs::default(),
        )
        .expect("second replace_text applies");
    assert_eq!(second_report.base_revision, 2);
    assert_eq!(second_report.new_revision, 3);
    assert_eq!(
        document
            .validate()
            .expect("final validation succeeds")
            .revision,
        3
    );
}

fn assert_add_image_dry_run_failure(
    bytes: &[u8],
    operation_override: serde_json::Value,
    media: MediaInputs,
    expected_code: ErrorCode,
    operation_id: &str,
) {
    let mut operation = serde_json::json!({
        "operation_id": operation_id,
        "op": "add_image",
        "slide_id": "slide-1",
        "media_ref": "hero",
        "content_type": "image/png",
        "bounds": { "x": 0, "y": 0, "cx": 914400, "cy": 914400 }
    });
    merge_object(&mut operation, operation_override);
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": operation_id,
        "operations": [operation]
    }))
    .expect("add_image patch parses");

    let mut document = PresentationDocument::from_bytes(bytes).expect("text deck opens");
    let error = document
        .apply_patch_with_options(
            patch,
            media,
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect_err("dry-run validation must fail");
    assert_eq!(error.code(), expected_code, "{error}");
    assert_eq!(
        error.details().location.operation_id.as_deref(),
        Some(operation_id)
    );
    assert_eq!(
        error.details().location.operation.as_deref(),
        Some("add_image")
    );
}

fn merge_object(target: &mut serde_json::Value, patch: serde_json::Value) {
    let target = target
        .as_object_mut()
        .expect("base operation is a JSON object");
    let patch = patch.as_object().expect("patch override is a JSON object");
    for (key, value) in patch {
        target.insert(key.clone(), value.clone());
    }
}

fn noop_patch(bytes: &[u8]) -> Patch {
    serde_json::from_value(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": "facade-api-noop",
        "operations": []
    }))
    .expect("noop patch parses")
}

fn replace_title_patch(
    document_id: &str,
    base_revision: u32,
    operation_id: &str,
    text: &str,
) -> Patch {
    parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id,
        "base_revision": base_revision,
        "client_request_id": operation_id,
        "operations": [{
            "operation_id": operation_id,
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": text
        }]
    }))
    .expect("replace title patch parses")
}

fn document_id(bytes: &[u8]) -> String {
    let entries = from_bytes(bytes).expect("package entries read");
    document_id_from_entries(&entries)
}

fn document_id_from_entries(entries: &[RawEntry]) -> String {
    let content_types_bytes = entries
        .iter()
        .find(|entry| !entry.meta.is_dir && entry.name.as_str() == "/[Content_Types].xml")
        .map(|entry| entry.bytes.as_slice())
        .expect("package has content types");
    let ordinary_parts = entries
        .iter()
        .filter(|entry| !entry.meta.is_dir && entry.name.as_str() != "/[Content_Types].xml")
        .map(|entry| (entry.name.clone(), entry.bytes.as_slice()))
        .collect::<Vec<_>>();

    provenance_document_id(&ordinary_parts, content_types_bytes)
}

fn media_inputs(media_ref: &str, content_type: &str, bytes: Vec<u8>) -> MediaInputs {
    let mut bindings = HashMap::new();
    bindings.insert(
        media_ref.to_owned(),
        MediaBinding {
            content_type: content_type.to_owned(),
            declared_sha256: None,
            declared_byte_length: None,
            source: MediaSource::Bytes(bytes),
        },
    );
    MediaInputs::new(bindings)
}

fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

fn entry_text(entries: &[RawEntry], zip_entry_name: &str) -> String {
    let entry = entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == zip_entry_name)
        .unwrap_or_else(|| panic!("{zip_entry_name} entry exists"));
    std::str::from_utf8(&entry.bytes)
        .unwrap_or_else(|error| panic!("{zip_entry_name} is UTF-8: {error}"))
        .to_owned()
}

fn unique_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    let base_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();

    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = temp_dir.join(format!(
            "pptx-compose-facade-api-{}-{base_nanos}-{id}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("test dir creates: {error}"),
        }
    }

    panic!("could not create a unique facade test directory")
}

#[allow(dead_code)]
fn _assert_no_internal_parser_types_in_primary_api(_path: &Path) {}

fn text_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", text_slide().as_bytes()),
        ],
        CompressionMethod::Stored,
    )
}

fn text_deck_with_directories() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.add_directory("ppt/", options).expect("add ppt dir");
        writer
            .add_directory("ppt/slides/", options)
            .expect("add slides dir");
        for (name, data) in [
            ("[Content_Types].xml", content_types().into_bytes()),
            ("_rels/.rels", root_rels().into_bytes()),
            ("ppt/presentation.xml", presentation().into_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().into_bytes(),
            ),
            ("ppt/slides/slide1.xml", text_slide().into_bytes()),
        ] {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(&data).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

fn zip_entries<const N: usize>(entries: [(&str, &[u8]); N], method: CompressionMethod) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, data) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(data).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

fn content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
        .to_owned()
}

fn root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
        .to_owned()
}

fn presentation() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

fn presentation_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#
        .to_owned()
}

fn text_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original title</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}
