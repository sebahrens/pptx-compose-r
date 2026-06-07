use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Cursor,
    io::Write,
    path::Path,
};

use pptx_compose::{
    AgentViewOptions, ApplyPatchOptions, MediaInputs, OpenOptions, Patch, PresentationDocument,
    ResourceLimits, ValidationMode, WriteMode, WriteOptions,
    edit::{
        media_inputs::{MediaBinding, MediaSource},
        patch::parse_patch,
    },
    json::agent_view::{
        FindTextScope,
        views::{FindTextRequest, ViewMode},
    },
    json::schemas::{OperationStatus, PatchStatus},
    part_checksum,
};
use pptx_compose_core::{
    error::ErrorCode,
    provenance::document_id::document_id as provenance_document_id,
    zip::reader::{RawEntry, from_bytes},
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
            atomic_temp_path: None,
            keep_temp: false,
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
    let _explicit_validation = from_path
        .validate_with_mode(ValidationMode::NoEdit)
        .expect("explicit validation report builds");
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
fn atomic_write_path_accepts_bare_relative_output() {
    use std::sync::{Mutex, OnceLock};

    static CURRENT_DIR_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    let _guard = CURRENT_DIR_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("current-dir test lock acquired");
    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let document = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let root = unique_dir();
    let previous_dir = std::env::current_dir().expect("current dir reads");
    std::env::set_current_dir(&root).expect("current dir changes to test root");

    document
        .write_path_with_options(
            Path::new("bare-output.pptx"),
            WriteOptions {
                overwrite: false,
                ..WriteOptions::default()
            },
        )
        .expect("bare relative atomic output succeeds");

    let output = root.join("bare-output.pptx");
    assert!(output.exists(), "bare relative output is published");
    let temp_prefix = ".bare-output.pptx.";
    let temp_remains = fs::read_dir(&root).expect("test root reads").any(|entry| {
        entry
            .expect("test root entry reads")
            .file_name()
            .to_string_lossy()
            .starts_with(temp_prefix)
    });
    assert!(
        !temp_remains,
        "successful atomic write removes temp sibling"
    );

    std::env::set_current_dir(previous_dir).expect("current dir restores");
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn open_options_apply_resource_limits_at_facade_boundary() {
    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let options = OpenOptions::with_resource_limits(ResourceLimits {
        max_compressed_package_bytes: u64::try_from(bytes.len() - 1)
            .expect("fixture length fits u64"),
        ..ResourceLimits::default()
    });

    let error = PresentationDocument::from_bytes_with_options(bytes, options)
        .expect_err("compressed package size limit must reject oversized input");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
    assert!(
        error.message().contains("maximum compressed size"),
        "{error}"
    );
}

#[test]
fn from_bytes_limit_rejects_oversized_non_zip_before_sniff() {
    let bytes = b"this is not a zip package";
    let options = OpenOptions::with_resource_limits(ResourceLimits {
        max_compressed_package_bytes: u64::try_from(bytes.len() - 1).expect("test length fits u64"),
        ..ResourceLimits::default()
    });

    let error = PresentationDocument::from_bytes_with_options(bytes, options)
        .expect_err("compressed package size limit must run before package sniffing");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
}

#[test]
fn compressed_limit_rejects_path_and_reader_before_zip_parse() {
    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let options = OpenOptions::with_resource_limits(ResourceLimits {
        max_compressed_package_bytes: u64::try_from(bytes.len() - 1)
            .expect("fixture length fits u64"),
        ..ResourceLimits::default()
    });
    let root = unique_dir();
    let input = root.join("input.pptx");
    fs::write(&input, bytes).expect("fixture writes");

    let path_error = PresentationDocument::open_path_with_options(&input, options.clone())
        .expect_err("path open must reject oversized compressed package");
    let reader_error = PresentationDocument::open_reader_with_options(Cursor::new(bytes), options)
        .expect_err("reader open must reject oversized compressed package");

    assert_eq!(path_error.code(), ErrorCode::ResourceLimitExceeded);
    assert_eq!(reader_error.code(), ErrorCode::ResourceLimitExceeded);
    fs::remove_dir_all(root).expect("test dir removes");
}

#[test]
fn seek_reader_sniffs_package_type_before_zip_parse() {
    let bytes = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00];

    let error = PresentationDocument::open_seek_reader_with_options(
        Cursor::new(bytes),
        OpenOptions::default(),
    )
    .expect_err("CFBF seek-reader input must be rejected before ZIP parsing");

    assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
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
fn validation_reports_duplicate_slide_ids_from_real_package() {
    let bytes = duplicate_slide_id_deck();
    let document = PresentationDocument::from_bytes(&bytes).expect("duplicate-id deck opens");

    let validation = document
        .validate_with_mode(ValidationMode::Edited)
        .expect("validation report builds");

    assert_eq!(
        validation.status,
        pptx_compose::json::schemas::ValidationStatus::Invalid
    );
    let finding = validation
        .findings
        .iter()
        .find(|finding| finding.code == pptx_compose::json::schemas::FindingCode::DuplicateSlideId)
        .expect("duplicate slide id finding is present");
    assert_eq!(finding.location["slide_id"], "256");
    assert_eq!(finding.location["relationship_id"], "rId1");
    assert_eq!(finding.location["part"], "ppt/slides/slide1.xml");
}

#[test]
fn no_edit_validation_reports_existing_errors_without_blocking() {
    let bytes = duplicate_slide_id_deck();
    let document = PresentationDocument::from_bytes(&bytes).expect("duplicate-id deck opens");

    let no_edit = document
        .validate_with_mode(ValidationMode::NoEdit)
        .expect("no-edit validation report builds");
    let edited = document
        .validate_with_mode(ValidationMode::Edited)
        .expect("edited validation report builds");

    assert_eq!(
        no_edit.status,
        pptx_compose::json::schemas::ValidationStatus::Valid
    );
    assert_eq!(no_edit.summary.errors, 1);
    assert!(no_edit.findings.iter().all(|finding| !finding.blocking));
    assert_eq!(
        edited.status,
        pptx_compose::json::schemas::ValidationStatus::Invalid
    );
    assert_eq!(edited.summary.errors, 1);
    assert!(edited.findings.iter().any(|finding| finding.blocking));
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
fn inspect_find_text_and_apply_use_identical_selector_guards() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let full_inspect = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlidePage,
            include_elements: true,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("full deck inspect builds");
    let scoped_inspect = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("scoped inspect builds");
    let find_hit = document
        .find_text(FindTextRequest {
            query: "Original".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        })
        .expect("find_text succeeds")
        .matches
        .into_iter()
        .next()
        .expect("text hit exists");

    let full_element = inspect_element(&full_inspect, &find_hit.element_id);
    let scoped_element = inspect_element(&scoped_inspect, &find_hit.element_id);
    let full_text_hash = full_element["text"]["text_hash"]
        .as_str()
        .expect("full inspect element has text hash");
    let scoped_text_hash = scoped_element["text"]["text_hash"]
        .as_str()
        .expect("scoped inspect element has text hash");
    let full_fingerprint = full_element["fingerprint"]
        .as_str()
        .expect("full inspect element has fingerprint");
    let scoped_fingerprint = scoped_element["fingerprint"]
        .as_str()
        .expect("scoped inspect element has fingerprint");

    assert_eq!(full_text_hash, find_hit.text_hash);
    assert_eq!(scoped_text_hash, find_hit.text_hash);
    assert_eq!(full_fingerprint, find_hit.fingerprint);
    assert_eq!(scoped_fingerprint, find_hit.fingerprint);

    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "inspect-guard-parity",
        "operations": [{
            "operation_id": "replace-from-inspect",
            "op": "replace_text",
            "selector": {
                "type": "element_id",
                "id": full_element["id"],
                "guards": {
                    "slide_id": full_element["slide_id"],
                    "kind": full_element["kind"],
                    "part": full_element["part"],
                    "text_hash": full_text_hash,
                    "fingerprint": full_fingerprint
                }
            },
            "text": "Updated from inspect guards"
        }]
    }))
    .expect("inspect-guarded patch parses");
    document
        .apply_patch(patch, MediaInputs::default())
        .expect("inspect-guarded patch applies");
}

#[test]
fn inspect_surfaces_set_alt_text_read_back() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let patch = patch_with_operations(
        &bytes,
        "set-alt-text-read-back",
        vec![serde_json::json!({
            "operation_id": "set-accessibility",
            "op": "set_alt_text",
            "element_id": "slide-1:shape-3",
            "title": "Accessible title",
            "description": "UNIQUE_MARKER accessible description"
        })],
    );
    document
        .apply_patch(patch, MediaInputs::default())
        .expect("set_alt_text applies");
    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    let reopened = PresentationDocument::from_bytes(&written).expect("edited deck reopens");
    let inspect = reopened
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("edited deck inspects");
    let element = inspect_element(&inspect, "slide-1:shape-3");

    assert_eq!(element["accessibility"]["title"], "Accessible title");
    assert_eq!(
        element["accessibility"]["description"],
        "UNIQUE_MARKER accessible description"
    );
}

#[test]
fn agent_view_rejects_huge_page_limit() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let error = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlidePage,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: Some(101),
        })
        .expect_err("huge agent view limit is rejected");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
}

#[test]
fn agent_view_advertises_image_content_types_accepted_by_image_edits() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let view = document.to_agent_json().expect("agent view builds");
    let advertised = view["capabilities"]["media_content_types"]
        .as_array()
        .expect("media content types are an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("media content type is a string")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    let accepted = ["image/png", "image/jpeg", "image/gif"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    assert_eq!(advertised, accepted);
}

#[test]
fn agent_view_advertises_set_document_metadata() {
    let document = PresentationDocument::from_bytes(metadata_deck()).expect("metadata deck opens");
    let view = document.to_agent_json().expect("agent view builds");
    let advertised = view["capabilities"]["operations"]
        .as_array()
        .expect("operations are an array")
        .iter()
        .map(|value| value.as_str().expect("operation is a string"))
        .collect::<BTreeSet<_>>();

    assert!(advertised.contains("set_document_metadata"));
}

#[test]
fn set_document_metadata_round_trips_and_preserves_clean_parts() {
    let bytes = metadata_deck();
    let original_entries = from_bytes(&bytes).expect("original entries read");
    let core_before = entry_text(&original_entries, "docProps/core.xml");
    let core_checksum = part_checksum(core_before.as_bytes());
    let mut document = PresentationDocument::from_bytes(&bytes).expect("metadata deck opens");
    let patch = patch_with_operations(
        &bytes,
        "set-document-metadata",
        vec![serde_json::json!({
            "operation_id": "set-core-fields",
            "op": "set_document_metadata",
            "selector": {
                "type": "core_properties",
                "part": "docProps/core.xml",
                "guards": {
                    "part_checksum": core_checksum
                }
            },
            "match": {
                "title": "Old title"
            },
            "metadata": {
                "title": "New title",
                "subject": "Board update",
                "creator": "Research Team",
                "keywords": "finance; q4"
            }
        })],
    );

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("metadata applies");
    assert_eq!(report.changed_parts, vec!["docProps/core.xml"]);
    assert_ne!(report.new_document_id, report.document_id);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    let reopened = PresentationDocument::from_bytes(&written).expect("edited deck reopens");
    assert_eq!(
        reopened.validate().expect("edited deck validates").status,
        pptx_compose::json::schemas::ValidationStatus::Valid
    );
    let written_entries = from_bytes(&written).expect("written entries read");
    assert_exact_part_deltas(
        "set_document_metadata",
        &original_entries,
        &written_entries,
        &["docProps/core.xml"],
        &[],
    );

    let core_after = entry_text(&written_entries, "docProps/core.xml");
    assert!(core_after.contains("<dc:title>New title</dc:title>"));
    assert!(core_after.contains("<dc:subject>Board update</dc:subject>"));
    assert!(core_after.contains("<dc:creator>Research Team</dc:creator>"));
    assert!(core_after.contains("<cp:keywords>finance; q4</cp:keywords>"));
    assert!(core_after.contains(
        r#"<dcterms:created xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:created>"#
    ));
    assert!(core_after.contains(
        r#"<dcterms:modified xsi:type="dcterms:W3CDTF">2026-01-02T00:00:00Z</dcterms:modified>"#
    ));
    assert!(core_after.contains("<cp:lastModifiedBy>Original editor</cp:lastModifiedBy>"));
}

#[test]
fn set_document_metadata_rejects_stale_revision_and_match_guard() {
    let bytes = metadata_deck();
    let mut stale_document = PresentationDocument::from_bytes(&bytes).expect("metadata deck opens");
    let stale_patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 2,
        "client_request_id": "stale-metadata",
        "operations": [{
            "operation_id": "set-stale-title",
            "op": "set_document_metadata",
            "selector": {
                "type": "core_properties",
                "part": "docProps/core.xml"
            },
            "metadata": {
                "title": "New title"
            }
        }]
    }))
    .expect("stale patch parses");
    let stale_error = stale_document
        .apply_patch(stale_patch, MediaInputs::default())
        .expect_err("stale metadata patch fails");
    assert_eq!(stale_error.code(), ErrorCode::StalePatch);

    let mut guarded_document =
        PresentationDocument::from_bytes(&bytes).expect("metadata deck opens");
    let guarded_patch = patch_with_operations(
        &bytes,
        "metadata-match-guard",
        vec![serde_json::json!({
            "operation_id": "set-guarded-title",
            "op": "set_document_metadata",
            "selector": {
                "type": "core_properties",
                "part": "docProps/core.xml"
            },
            "match": {
                "title": "Not the current title"
            },
            "metadata": {
                "title": "New title"
            }
        })],
    );
    let guarded_report = guarded_document
        .apply_patch(guarded_patch, MediaInputs::default())
        .expect("metadata match guard returns a failed report");
    assert_eq!(guarded_report.status, PatchStatus::Failed);
    let guarded_error = guarded_report.operation_reports[0]
        .error
        .as_ref()
        .expect("failed operation has an error");
    assert_eq!(
        guarded_error.code,
        pptx_compose::json::schemas::ErrorCode::SelectorGuardFailed
    );
    assert_eq!(guarded_error.location["operation_id"], "set-guarded-title");
    assert_eq!(guarded_error.location["operation"], "set_document_metadata");
}

#[test]
fn find_text_pages_many_matches_without_unbounded_page() {
    let document = PresentationDocument::from_bytes(repeated_text_deck(150)).expect("deck opens");
    let first = document
        .find_text(FindTextRequest {
            query: "a".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: Some(100),
        })
        .expect("first page succeeds");

    assert_eq!(first.matches.len(), 100);
    assert!(first.view.truncated);
    assert_eq!(first.omitted_count, 1);
    assert_eq!(first.matches[0].span.start, 0);
    assert_eq!(first.matches[99].span.start, 99);

    let second = document
        .find_text(FindTextRequest {
            query: "a".to_owned(),
            scope: FindTextScope::Deck,
            cursor: first.view.next_cursor,
            limit: Some(100),
        })
        .expect("second page succeeds");

    assert_eq!(second.matches.len(), 50);
    assert!(!second.view.truncated);
    assert_eq!(second.omitted_count, 0);
    assert_eq!(second.matches[0].span.start, 100);
    assert_eq!(second.matches[49].span.start, 149);
}

#[test]
fn find_text_rejects_huge_page_limit() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let error = document
        .find_text(FindTextRequest {
            query: "Original".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: Some(101),
        })
        .expect_err("huge find_text limit is rejected");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
}

#[test]
fn agent_text_view_does_not_emit_unaddressable_run_or_paragraph_ids() {
    let document = PresentationDocument::from_bytes(text_deck()).expect("text deck opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
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
    assert_ne!(report.new_document_id, report.document_id);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    assert_ne!(written, bytes);
    assert_eq!(report.new_document_id, document_id(&written));

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
fn replace_text_dry_run_reports_effects_without_mutating_document() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "replace-text-dry-run",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "match": "Original title",
            "text": "Dry-run title"
        }]
    }))
    .expect("patch parses");

    let output = document
        .apply_patch_with_diff(
            patch,
            MediaInputs::default(),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("replace_text dry-run succeeds");

    assert_eq!(
        output.report.status,
        pptx_compose::json::schemas::PatchStatus::DryRunSuccess
    );
    assert_eq!(output.report.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert_ne!(output.report.new_document_id, output.report.document_id);
    assert_eq!(output.report.operation_reports.len(), 1);
    let operation = &output.report.operation_reports[0];
    assert_eq!(
        operation.status,
        pptx_compose::json::schemas::OperationStatus::Validated
    );
    assert_eq!(operation.target.element_id, "slide-1:shape-3");
    assert_eq!(operation.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert_eq!(output.diff.changed_parts.len(), 1);
    assert_eq!(output.diff.changed_parts[0].part, "ppt/slides/slide1.xml");
    assert_eq!(output.diff.changes.len(), 1);

    let written = document.write_vec().expect("document writes after dry-run");
    assert_eq!(written, bytes, "dry-run must not mutate document bytes");
}

#[test]
fn guarded_selector_replace_text_applies_and_rejects_stale_fingerprint() {
    let bytes = text_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let hit = document
        .find_text(FindTextRequest {
            query: "Original".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        })
        .expect("find_text succeeds")
        .matches
        .into_iter()
        .next()
        .expect("text hit exists");

    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "guarded-replace-text",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "selector": {
                "type": "element_id",
                "id": hit.element_id,
                "guards": {
                    "slide_id": hit.slide_id,
                    "kind": hit.kind,
                    "part": hit.part,
                    "text_hash": hit.text_hash,
                    "fingerprint": hit.fingerprint
                }
            },
            "text": "Guarded title"
        }]
    }))
    .expect("guarded patch parses");

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("matching guarded selector applies");
    assert_eq!(report.changed_parts, vec!["ppt/slides/slide1.xml"]);

    let mut stale_document = PresentationDocument::from_bytes(&bytes).expect("text deck opens");
    let stale_patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(&bytes),
        "base_revision": 1,
        "client_request_id": "guarded-replace-text",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "selector": {
                "type": "element_id",
                "id": "slide-1:shape-3",
                "guards": {
                    "slide_id": "slide-1",
                    "kind": "text_box",
                    "part": "ppt/slides/slide1.xml",
                    "fingerprint": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                }
            },
            "text": "Guarded title"
        }]
    }))
    .expect("stale guarded patch parses");

    let report = stale_document
        .apply_patch(stale_patch, MediaInputs::default())
        .expect("mismatched fingerprint guard returns a failed report");
    assert_eq!(report.status, PatchStatus::Failed);
    let error = report.operation_reports[0]
        .error
        .as_ref()
        .expect("failed operation has an error");
    assert_eq!(
        error.code,
        pptx_compose::json::schemas::ErrorCode::SelectorGuardFailed
    );
    assert_eq!(error.location["operation_id"], "replace-title");
    assert_eq!(error.location["element_id"], "slide-1:shape-3");
}

#[test]
fn inspect_element_guards_resolve_for_replace_text_dry_run() {
    let bytes = include_bytes!("../../../fixtures/real-world/worldbank-macro-economic-update.pptx");
    let document = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-2".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("slide_detail builds");
    let element = view["slides"][0]["elements"]
        .as_array()
        .expect("slide_detail exposes elements")
        .iter()
        .find(|element| {
            element["editable"]["text"]["supported"]
                .as_bool()
                .unwrap_or(false)
                && element["text"]["plain"]
                    .as_str()
                    .is_some_and(|plain| plain == "Authors")
        })
        .expect("slide-2 Authors element exists");

    let mut editable = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let patch = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": "inspect-guard-parity",
        "operations": [{
            "operation_id": "replace-authors",
            "op": "replace_text",
            "selector": {
                "type": "element_id",
                "id": element["id"],
                "guards": {
                    "slide_id": element["slide_id"],
                    "kind": element["kind"],
                    "part": element["part"],
                    "text_hash": element["text"]["text_hash"],
                    "fingerprint": element["fingerprint"]
                }
            },
            "match": element["text"]["plain"],
            "text": "Authors"
        }]
    }))
    .expect("inspect-guarded patch parses");

    let output = editable
        .apply_patch_with_diff(
            patch,
            MediaInputs::default(),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("inspect-derived guarded selector dry-run succeeds");

    assert_eq!(output.report.status, PatchStatus::DryRunSuccess);
    assert_eq!(output.report.changed_parts, vec!["ppt/slides/slide2.xml"]);
    assert_eq!(
        output.report.operation_reports[0].target.element_id,
        "slide-2:shape-2"
    );
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
fn v1_edit_operations_round_trip_with_exact_dirty_parts() {
    assert_successful_edit_round_trip(
        "replace_text",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": "Round-tripped title"
        }),
        MediaInputs::default(),
        &["ppt/slides/slide1.xml"],
        &[],
    );
    assert_successful_edit_round_trip(
        "add_text_box",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "add-caption",
            "op": "add_text_box",
            "slide_id": "slide-1",
            "text": "New caption",
            "bounds": { "x": 1828800, "y": 1828800, "cx": 1828800, "cy": 457200 },
            "name": "Caption",
            "alt_text": "Generated caption"
        }),
        MediaInputs::default(),
        &["ppt/slides/slide1.xml"],
        &[],
    );
    assert_successful_edit_round_trip(
        "move_resize_element",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "move-title",
            "op": "move_resize_element",
            "element_id": "slide-1:shape-3",
            "bounds": { "x": 457200, "y": 914400, "cx": 2743200, "cy": 685800 }
        }),
        MediaInputs::default(),
        &["ppt/slides/slide1.xml"],
        &[],
    );
    assert_successful_edit_round_trip(
        "set_alt_text",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "alt-title",
            "op": "set_alt_text",
            "element_id": "slide-1:shape-3",
            "title": "Accessible title",
            "description": "Updated accessible description"
        }),
        MediaInputs::default(),
        &["ppt/slides/slide1.xml"],
        &[],
    );
    assert_successful_edit_round_trip(
        "add_image",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "add-thumb",
            "op": "add_image",
            "slide_id": "slide-1",
            "media_ref": "new-image",
            "content_type": "image/png",
            "bounds": { "x": 0, "y": 2743200, "cx": 914400, "cy": 914400 },
            "name": "Thumbnail",
            "alt_text": "Thumbnail image"
        }),
        media_inputs("new-image", "image/png", tiny_png()),
        &[
            "ppt/media/image2.png",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/slide1.xml",
        ],
        &["ppt/media/image2.png", "ppt/slides/_rels/slide1.xml.rels"],
    );
    assert_successful_edit_round_trip(
        "replace_image",
        image_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "replace-hero",
            "op": "replace_image",
            "element_id": "slide-1:pic-4",
            "media_ref": "replacement-image",
            "content_type": "image/png"
        }),
        media_inputs("replacement-image", "image/png", tiny_png()),
        &["ppt/media/image2.png", "ppt/slides/_rels/slide1.xml.rels"],
        &["ppt/media/image2.png"],
    );
}

#[test]
fn gif_image_edits_round_trip_with_gif_media_parts() {
    assert_successful_edit_round_trip(
        "add_image_gif",
        text_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "add-gif",
            "op": "add_image",
            "slide_id": "slide-1",
            "media_ref": "new-gif",
            "content_type": "image/gif",
            "bounds": { "x": 0, "y": 2743200, "cx": 914400, "cy": 914400 },
            "name": "Animated marker",
            "alt_text": "GIF marker"
        }),
        media_inputs("new-gif", "image/gif", tiny_gif()),
        &[
            "[Content_Types].xml",
            "ppt/media/image1.gif",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/slide1.xml",
        ],
        &["ppt/media/image1.gif", "ppt/slides/_rels/slide1.xml.rels"],
    );
    assert_successful_edit_round_trip(
        "replace_image_gif",
        image_deck_with_clean_extras(),
        serde_json::json!({
            "operation_id": "replace-gif",
            "op": "replace_image",
            "element_id": "slide-1:pic-4",
            "media_ref": "replacement-gif",
            "content_type": "image/gif"
        }),
        media_inputs("replacement-gif", "image/gif", tiny_gif()),
        &[
            "[Content_Types].xml",
            "ppt/media/image1.gif",
            "ppt/slides/_rels/slide1.xml.rels",
        ],
        &["ppt/media/image1.gif"],
    );
}

#[test]
fn external_link_image_view_flag_matches_replace_image_rejection() {
    let bytes = linked_image_deck();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("linked image deck opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("slide_detail builds");
    let linked = view["slides"][0]["elements"]
        .as_array()
        .expect("slide_detail exposes elements")
        .iter()
        .find(|element| element["id"] == "slide-1:pic-4")
        .expect("linked picture is projected");

    assert_eq!(linked["editable"]["image"]["supported"], false);
    assert_eq!(linked["editable"]["image"]["reason"], "external_link");
    assert_eq!(linked.get("image"), None);

    let patch = patch_with_operations(
        &bytes,
        "replace-linked-image",
        vec![serde_json::json!({
            "operation_id": "replace-linked",
            "op": "replace_image",
            "element_id": "slide-1:pic-4",
            "media_ref": "replacement",
            "content_type": "image/png"
        })],
    );
    let report = document
        .apply_patch(patch, media_inputs("replacement", "image/png", tiny_png()))
        .expect("replace_image rejection returns a failed report");

    assert_eq!(report.status, PatchStatus::Failed);
    let error = report.operation_reports[0]
        .error
        .as_ref()
        .expect("failed operation has an error");
    assert_eq!(
        error.code,
        pptx_compose::json::schemas::ErrorCode::UnsupportedEdit
    );
    assert_eq!(error.location["operation_id"], "replace-linked");
    assert_eq!(error.location["element_id"], "slide-1:pic-4");
}

#[test]
fn text_editability_view_flag_matches_replace_text_acceptance() {
    let bytes = graphic_frame_deck();
    let document = PresentationDocument::from_bytes(&bytes).expect("graphic frame deck opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("slide_detail builds");
    let elements = view["slides"][0]["elements"]
        .as_array()
        .expect("slide_detail exposes elements");

    for element in elements {
        let element_id = element["id"].as_str().expect("element id is a string");
        let advertised = element["editable"]
            .get("text")
            .map(|text| {
                text["supported"]
                    .as_bool()
                    .expect("text edit support is a bool")
            })
            .unwrap_or(false);
        let mut candidate = PresentationDocument::from_bytes(&bytes).expect("candidate deck opens");
        let patch = patch_with_operations(
            &bytes,
            &format!("replace-text-capability-{element_id}"),
            vec![serde_json::json!({
                "operation_id": format!("replace-{element_id}"),
                "op": "replace_text",
                "element_id": element_id,
                "text": "replacement"
            })],
        );
        let accepted = candidate
            .apply_patch(patch, MediaInputs::default())
            .map(|report| report.status == PatchStatus::Applied)
            .unwrap_or(false);

        assert_eq!(
            advertised, accepted,
            "{element_id}: editable.text.supported must match replace_text acceptance"
        );
    }
}

#[test]
fn graphic_frame_kind_view_and_no_edit_round_trip_are_stable() {
    let bytes = graphic_frame_deck();
    let document = PresentationDocument::from_bytes(&bytes).expect("graphic frame deck opens");
    let view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("slide_detail builds");
    let elements = view["slides"][0]["elements"]
        .as_array()
        .expect("slide_detail exposes elements");

    for (element_id, kind) in [
        ("slide-1:graphic-7", "chart"),
        ("slide-1:graphic-8", "table"),
        ("slide-1:graphic-9", "diagram"),
        ("slide-1:graphic-10", "ole"),
        ("slide-1:graphic-11", "shape"),
    ] {
        let element = elements
            .iter()
            .find(|element| element["id"] == element_id)
            .unwrap_or_else(|| panic!("{element_id} should be projected"));
        assert_eq!(element["kind"], kind);
    }

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("no-edit graphic frame deck writes");
    let original_entries = from_bytes(&bytes).expect("original entries read");
    let written_entries = from_bytes(&written).expect("written entries read");
    assert_eq!(
        entry_bytes(&written_entries, "ppt/slides/slide1.xml"),
        entry_bytes(&original_entries, "ppt/slides/slide1.xml"),
        "read-only graphic frame reclassification must not dirty slide XML"
    );
}

#[test]
fn table_and_diagram_graphic_frames_accept_bounds_and_alt_text_edits() {
    for (operation_name, element_id, operation) in [
        (
            "move_resize_table_frame",
            "slide-1:graphic-8",
            serde_json::json!({
                "operation_id": "move-table",
                "op": "move_resize_element",
                "element_id": "slide-1:graphic-8",
                "bounds": { "x": 457200, "y": 457200, "cx": 1828800, "cy": 914400 }
            }),
        ),
        (
            "move_resize_diagram_frame",
            "slide-1:graphic-9",
            serde_json::json!({
                "operation_id": "move-diagram",
                "op": "move_resize_element",
                "element_id": "slide-1:graphic-9",
                "bounds": { "x": 2286000, "y": 457200, "cx": 1828800, "cy": 914400 }
            }),
        ),
        (
            "set_alt_text_table_frame",
            "slide-1:graphic-8",
            serde_json::json!({
                "operation_id": "alt-table",
                "op": "set_alt_text",
                "element_id": "slide-1:graphic-8",
                "title": "Table frame",
                "description": "Accessible table frame"
            }),
        ),
        (
            "set_alt_text_diagram_frame",
            "slide-1:graphic-9",
            serde_json::json!({
                "operation_id": "alt-diagram",
                "op": "set_alt_text",
                "element_id": "slide-1:graphic-9",
                "title": "Diagram frame",
                "description": "Accessible diagram frame"
            }),
        ),
    ] {
        assert_eq!(operation["element_id"], element_id);
        assert_successful_edit_round_trip(
            operation_name,
            graphic_frame_deck_with_clean_extras(),
            operation,
            MediaInputs::default(),
            &["ppt/slides/slide1.xml"],
            &[],
        );
    }
}

#[test]
fn replace_table_cell_text_edits_non_merged_cell_preserving_run_formatting() {
    let bytes = graphic_frame_deck_with_clean_extras();
    let original_entries = from_bytes(&bytes).expect("original entries read");
    let original_slide = entry_text(&original_entries, "ppt/slides/slide1.xml");
    let original_tbl_grid = extract_between(&original_slide, "<a:tblGrid>", "</a:tblGrid>");
    let mut document = PresentationDocument::from_bytes(&bytes).expect("fixture opens");
    let patch = patch_with_operations(
        &bytes,
        "replace-table-cell-text",
        vec![serde_json::json!({
            "operation_id": "replace-table-cell",
            "op": "replace_text",
            "element_id": "slide-1:graphic-8",
            "cell": { "row": 0, "col": 0 },
            "text": "Northwest",
            "match": "North"
        })],
    );

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("replace_text table cell applies");
    assert_eq!(report.changed_parts, vec!["ppt/slides/slide1.xml"]);

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    let written_entries = from_bytes(&written).expect("written entries read");
    assert_exact_part_deltas(
        "replace_text",
        &original_entries,
        &written_entries,
        &["ppt/slides/slide1.xml"],
        &[],
    );
    let written_slide = entry_text(&written_entries, "ppt/slides/slide1.xml");
    assert!(
        written_slide.contains(r#"<a:rPr b="1"/><a:t>Northwest</a:t>"#),
        "cell run properties should be preserved without fabricated rPr"
    );
    assert!(
        written_slide.contains("<a:t>East</a:t>")
            && written_slide.contains("<a:t>South</a:t>")
            && written_slide.contains("<a:t>West</a:t>"),
        "sibling cells should be unchanged"
    );
    assert_eq!(
        extract_between(&written_slide, "<a:tblGrid>", "</a:tblGrid>"),
        original_tbl_grid,
        "replace_table_cell_text must not modify a:tblGrid"
    );
}

#[test]
fn replace_text_edits_speaker_notes_via_slide_selector() {
    let bytes = notes_deck_with_clean_extras();
    let original_entries = from_bytes(&bytes).expect("original entries read");
    let mut document = PresentationDocument::from_bytes(&bytes).expect("fixture opens");
    let patch = patch_with_operations(
        &bytes,
        "replace-notes-text",
        vec![serde_json::json!({
            "operation_id": "replace-notes",
            "op": "replace_text",
            "slide_id": "slide-1",
            "run": { "paragraph_index": 0, "run_index": 0 },
            "text": "Updated speaker notes",
            "match": "Original speaker notes"
        })],
    );

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("replace_text notes target applies");
    assert_eq!(
        report.changed_parts,
        vec!["ppt/notesSlides/notesSlide1.xml"]
    );

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("edited deck writes");
    let written_entries = from_bytes(&written).expect("written entries read");
    assert_exact_part_deltas(
        "replace_text notes",
        &original_entries,
        &written_entries,
        &["ppt/notesSlides/notesSlide1.xml"],
        &[],
    );
    let written_notes = entry_text(&written_entries, "ppt/notesSlides/notesSlide1.xml");
    assert!(written_notes.contains("<a:t>Updated speaker notes</a:t>"));
    assert!(!written_notes.contains("<a:t>Original speaker notes</a:t>"));
}

#[test]
fn replace_table_cell_text_rejects_merged_or_spanned_cells() {
    let slide =
        graphic_frame_slide().replacen("<a:tc><a:txBody>", r#"<a:tc gridSpan="2"><a:txBody>"#, 1);
    let bytes = text_deck_with_slide(&slide);
    let mut document = PresentationDocument::from_bytes(&bytes).expect("fixture opens");
    let patch = patch_with_operations(
        &bytes,
        "replace-merged-table-cell-text",
        vec![serde_json::json!({
            "operation_id": "replace-merged-table-cell",
            "op": "replace_text",
            "element_id": "slide-1:graphic-8",
            "cell": { "row": 0, "col": 0 },
            "text": "Merged"
        })],
    );

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("merged table cell edit returns a failed report");
    assert_eq!(report.status, PatchStatus::Failed);
    assert_eq!(report.operation_reports[0].status, OperationStatus::Failed);
    assert_eq!(
        report.operation_reports[0]
            .error
            .as_ref()
            .expect("failed operation has an error")
            .code,
        pptx_compose::json::schemas::ErrorCode::UnsupportedEdit
    );

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("failed edit deck writes");
    assert_eq!(written, bytes);
}

#[test]
fn failed_multi_operation_patch_leaves_package_byte_identical() {
    let bytes = text_deck_with_clean_extras();
    let mut document = PresentationDocument::from_bytes(&bytes).expect("fixture opens");
    let patch = patch_with_operations(
        &bytes,
        "failed-atomic-patch",
        vec![
            serde_json::json!({
                "operation_id": "replace-before-failure",
                "op": "replace_text",
                "element_id": "slide-1:shape-3",
                "text": "This must not persist"
            }),
            serde_json::json!({
                "operation_id": "invalid-alt-text",
                "op": "set_alt_text",
                "element_id": "slide-1:shape-3"
            }),
        ],
    );

    let report = document
        .apply_patch(patch, MediaInputs::default())
        .expect("multi-operation patch returns a failed report");
    assert_eq!(report.status, PatchStatus::Failed);
    assert_eq!(report.changed_parts, Vec::<String>::new());
    assert_eq!(report.operation_reports[0].status, OperationStatus::Applied);
    assert_eq!(report.operation_reports[1].status, OperationStatus::Failed);
    assert_eq!(
        report.operation_reports[1]
            .error
            .as_ref()
            .expect("failed operation has an error")
            .code,
        pptx_compose::json::schemas::ErrorCode::InvalidInput
    );

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .expect("unchanged deck writes");
    assert_eq!(written, bytes);
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
    assert_add_image_patch_parse_failure(
        &bytes,
        serde_json::json!({
            "fit": "contain"
        }),
        "unsupported-fit",
    );
    assert_add_image_patch_parse_failure(
        &bytes,
        serde_json::json!({
            "dedupe": "checksum"
        }),
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
    let after_first_view = document
        .to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: false,
            slide_id: Some("slide-1".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })
        .expect("post-apply agent view builds");
    assert_eq!(after_first_view["revision"], 2);
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
                u32::try_from(
                    after_first_view["revision"]
                        .as_u64()
                        .expect("agent view revision is numeric"),
                )
                .expect("agent view revision fits patch schema"),
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
    let report = document
        .apply_patch_with_options(
            patch,
            media,
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("dry-run validation returns a failed report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::DryRunFailed
    );
    assert_eq!(report.operation_reports.len(), 1);
    let operation_error = report.operation_reports[0]
        .error
        .as_ref()
        .expect("failed operation reports an error");
    assert_eq!(
        serde_json::to_value(operation_error.code).expect("error code serializes"),
        serde_json::to_value(expected_code).expect("expected error code serializes")
    );
    assert_eq!(
        operation_error.location["operation_id"],
        serde_json::json!(operation_id)
    );
    assert_eq!(operation_error.location["operation"], "add_image");
}

fn assert_add_image_patch_parse_failure(
    bytes: &[u8],
    operation_override: serde_json::Value,
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
    let error = parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": operation_id,
        "operations": [operation]
    }))
    .expect_err("unsupported add_image schema field is rejected before dry-run");

    assert_eq!(error.code(), ErrorCode::InvalidInput);
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

fn assert_successful_edit_round_trip(
    operation_name: &str,
    bytes: Vec<u8>,
    operation: serde_json::Value,
    media: MediaInputs,
    expected_changed_parts: &[&str],
    expected_added_parts: &[&str],
) {
    let mut document = PresentationDocument::from_bytes(&bytes).expect("fixture opens");
    let patch = patch_with_operations(&bytes, operation_name, vec![operation]);

    let report = document
        .apply_patch(patch, media)
        .unwrap_or_else(|error| panic!("{operation_name} applies: {error}"));
    assert_part_list_eq(
        &report.changed_parts,
        expected_changed_parts,
        operation_name,
        "report.changed_parts",
    );
    assert!(
        !report.changed_parts.is_empty(),
        "{operation_name}: changed_parts must be non-empty"
    );

    let written = document
        .write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })
        .unwrap_or_else(|error| panic!("{operation_name} edited deck writes: {error}"));
    let reopened = PresentationDocument::from_bytes(&written)
        .unwrap_or_else(|error| panic!("{operation_name} written deck reopens: {error}"));
    let validation = reopened
        .validate()
        .unwrap_or_else(|error| panic!("{operation_name} written deck validates: {error}"));
    assert_eq!(
        validation.status,
        pptx_compose::json::schemas::ValidationStatus::Valid,
        "{operation_name}: validation failed"
    );

    let original_entries = from_bytes(&bytes).expect("original entries read");
    let written_entries = from_bytes(&written).expect("written entries read");
    assert_exact_part_deltas(
        operation_name,
        &original_entries,
        &written_entries,
        expected_changed_parts,
        expected_added_parts,
    );
}

fn patch_with_operations(
    bytes: &[u8],
    client_request_id: &str,
    operations: Vec<serde_json::Value>,
) -> Patch {
    parse_patch(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": client_request_id,
        "operations": operations
    }))
    .expect("patch parses")
}

fn assert_exact_part_deltas(
    operation_name: &str,
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    expected_changed_parts: &[&str],
    expected_added_parts: &[&str],
) {
    let expected_changed = part_set(expected_changed_parts);
    let expected_added = part_set(expected_added_parts);
    assert!(
        expected_added.is_subset(&expected_changed),
        "{operation_name}: added parts must also be reported as changed"
    );

    let original_by_name = entries_by_zip_name(original_entries);
    let written_by_name = entries_by_zip_name(written_entries);
    let original_names = original_by_name.keys().cloned().collect::<BTreeSet<_>>();
    let written_names = written_by_name.keys().cloned().collect::<BTreeSet<_>>();
    let added = written_names
        .difference(&original_names)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(added, expected_added, "{operation_name}: added part set");

    for name in original_names {
        let original = original_by_name
            .get(&name)
            .expect("original entry is indexed");
        let written = written_by_name
            .get(&name)
            .unwrap_or_else(|| panic!("{operation_name}: {name} was dropped"));
        if expected_changed.contains(name.as_str()) {
            assert_ne!(
                written.bytes, original.bytes,
                "{operation_name}: expected {name} to change"
            );
        } else {
            assert_eq!(
                written.bytes, original.bytes,
                "{operation_name}: clean part {name} changed"
            );
        }
    }
}

fn assert_part_list_eq(actual: &[String], expected: &[&str], operation_name: &str, label: &str) {
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{operation_name}: {label}");
}

fn part_set(parts: &[&str]) -> BTreeSet<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn entries_by_zip_name(entries: &[RawEntry]) -> BTreeMap<String, &RawEntry> {
    entries
        .iter()
        .map(|entry| (entry.name.zip_entry_name().to_owned(), entry))
        .collect()
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

fn inspect_element<'a>(view: &'a Value, element_id: &str) -> &'a Value {
    view["slides"]
        .as_array()
        .expect("inspect view has slides")
        .iter()
        .flat_map(|slide| {
            slide["elements"]
                .as_array()
                .expect("inspect slide has elements")
        })
        .find(|element| element["id"] == element_id)
        .unwrap_or_else(|| panic!("inspect view should include {element_id}"))
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

fn tiny_gif() -> Vec<u8> {
    vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02,
        0x02, 0x44, 0x01, 0x00, 0x3b,
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

fn entry_bytes<'a>(entries: &'a [RawEntry], zip_entry_name: &str) -> &'a [u8] {
    entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == zip_entry_name)
        .unwrap_or_else(|| panic!("{zip_entry_name} entry exists"))
        .bytes
        .as_slice()
}

fn extract_between(text: &str, start: &str, end: &str) -> String {
    let start_index = text
        .find(start)
        .unwrap_or_else(|| panic!("{start} marker exists"));
    let body_start = start_index + start.len();
    let body_end = text[body_start..]
        .find(end)
        .map(|offset| body_start + offset)
        .unwrap_or_else(|| panic!("{end} marker exists"));
    text[body_start..body_end].to_owned()
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
    text_deck_with_slide(&text_slide())
}

fn repeated_text_deck(count: usize) -> Vec<u8> {
    text_deck_with_slide(&text_slide_with_text(&"a".repeat(count)))
}

fn text_deck_with_slide(slide_xml: &str) -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", slide_xml.as_bytes()),
        ],
        CompressionMethod::Stored,
    )
}

fn text_deck_with_clean_extras() -> Vec<u8> {
    zip_entries(
        [
            (
                "[Content_Types].xml",
                content_types_with_png_and_unknown().as_bytes(),
            ),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", text_slide().as_bytes()),
            ("ppt/media/image1.png", &tiny_png()),
            ("custom/unknown.bin", b"unknown payload"),
        ],
        CompressionMethod::Stored,
    )
}

fn metadata_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types_with_core().as_bytes()),
            ("_rels/.rels", root_rels_with_core().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", text_slide().as_bytes()),
            ("docProps/core.xml", core_properties().as_bytes()),
            (
                "docProps/app.xml",
                b"<Properties><Application>PowerPoint</Application></Properties>",
            ),
            ("custom/unknown.bin", b"unknown payload"),
        ],
        CompressionMethod::Stored,
    )
}

fn image_deck_with_clean_extras() -> Vec<u8> {
    zip_entries(
        [
            (
                "[Content_Types].xml",
                content_types_with_png_and_unknown().as_bytes(),
            ),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", image_slide().as_bytes()),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                image_slide_rels().as_bytes(),
            ),
            ("ppt/media/image1.png", &tiny_png()),
            ("custom/unknown.bin", b"unknown payload"),
        ],
        CompressionMethod::Stored,
    )
}

fn graphic_frame_deck() -> Vec<u8> {
    text_deck_with_slide(&graphic_frame_slide())
}

fn graphic_frame_deck_with_clean_extras() -> Vec<u8> {
    zip_entries(
        [
            (
                "[Content_Types].xml",
                content_types_with_png_and_unknown().as_bytes(),
            ),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", graphic_frame_slide().as_bytes()),
            ("ppt/media/image1.png", &tiny_png()),
            ("custom/unknown.bin", b"unknown payload"),
        ],
        CompressionMethod::Stored,
    )
}

fn notes_deck_with_clean_extras() -> Vec<u8> {
    zip_entries(
        [
            (
                "[Content_Types].xml",
                content_types_with_notes_and_unknown().as_bytes(),
            ),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", notes_linked_slide().as_bytes()),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                notes_slide_rels().as_bytes(),
            ),
            ("ppt/notesSlides/notesSlide1.xml", notes_slide().as_bytes()),
            ("custom/unknown.bin", b"unknown payload"),
        ],
        CompressionMethod::Stored,
    )
}

fn linked_image_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            ("ppt/presentation.xml", presentation().as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                presentation_rels().as_bytes(),
            ),
            ("ppt/slides/slide1.xml", linked_image_slide().as_bytes()),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                linked_image_slide_rels().as_bytes(),
            ),
        ],
        CompressionMethod::Stored,
    )
}

fn duplicate_slide_id_deck() -> Vec<u8> {
    zip_entries(
        [
            ("[Content_Types].xml", content_types().as_bytes()),
            ("_rels/.rels", root_rels().as_bytes()),
            (
                "ppt/presentation.xml",
                presentation_with_duplicate_slide_ids().as_bytes(),
            ),
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

fn content_types_with_png_and_unknown() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Default Extension="bin" ContentType="application/octet-stream"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
        .to_owned()
}

fn content_types_with_notes_and_unknown() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="bin" ContentType="application/octet-stream"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>
</Types>"#
        .to_owned()
}

fn content_types_with_core() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="bin" ContentType="application/octet-stream"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
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

fn root_rels_with_core() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#
        .to_owned()
}

fn core_properties() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Old title</dc:title>
  <dcterms:created xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:created>
  <cp:lastModifiedBy>Original editor</cp:lastModifiedBy>
  <dcterms:modified xsi:type="dcterms:W3CDTF">2026-01-02T00:00:00Z</dcterms:modified>
</cp:coreProperties>"#
        .to_owned()
}

fn presentation() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

fn presentation_with_duplicate_slide_ids() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
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

fn image_slide_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#
        .to_owned()
}

fn linked_image_slide_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.test/image.png" TargetMode="External"/>
</Relationships>"#
        .to_owned()
}

fn notes_slide_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
</Relationships>"#
        .to_owned()
}

fn text_slide() -> String {
    text_slide_with_text("Original title")
}

fn notes_linked_slide() -> String {
    r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#
        .to_owned()
}

fn notes_slide() -> String {
    r#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes Placeholder 1"/><p:cNvSpPr txBox="1"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original speaker notes</a:t></a:r><a:r><a:t>Sibling note</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
        .to_owned()
}

fn text_slide_with_text(text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
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

fn image_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original title</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="4" name="Hero"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId2"/></p:blipFill>
        <p:spPr><a:xfrm><a:off x="0" y="1828800"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
      </p:pic>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="5" name="Shared Hero"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId3"/></p:blipFill>
        <p:spPr><a:xfrm><a:off x="914400" y="1828800"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
      </p:pic>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}

fn graphic_frame_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="7" name="Chart Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="0" y="457200"/><a:ext cx="1371600" cy="914400"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic>
      </p:graphicFrame>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="8" name="Table Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="1371600" y="457200"/><a:ext cx="1371600" cy="914400"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1"/><a:tblGrid><a:gridCol w="914400"/><a:gridCol w="914400"/></a:tblGrid><a:tr h="457200"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr b="1"/><a:t>North</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>East</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr><a:tr h="457200"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>South</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>West</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic>
      </p:graphicFrame>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="9" name="Diagram Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="2743200" y="457200"/><a:ext cx="1371600" cy="914400"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic>
      </p:graphicFrame>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="10" name="OLE Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="4114800" y="457200"/><a:ext cx="1371600" cy="914400"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole"/></a:graphic>
      </p:graphicFrame>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="11" name="Unknown Graphic Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="5486400" y="457200"/><a:ext cx="1371600" cy="914400"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://example.invalid/customGraphic"/></a:graphic>
      </p:graphicFrame>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}

fn linked_image_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="4" name="Linked Hero"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:link="rIdLink"/></p:blipFill>
        <p:spPr><a:xfrm><a:off x="0" y="1828800"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
      </p:pic>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}
