use std::collections::{BTreeMap, BTreeSet, HashMap};

use pptx_compose::{AgentViewOptions, MediaInputs, PresentationDocument, WriteMode, WriteOptions};
use pptx_compose_core::{
    error::{Error, Result},
    opc::{package::Package, part_name::PartName},
    validation::{Severity, ValidationMode, ValidationStatus, validate_package},
    zip::reader::{RawEntry, from_bytes},
};
use pptx_compose_edit::{
    media_inputs::{MediaBinding, MediaSource},
    patch::parse_patch,
};
use pptx_compose_json::agent_view::views::ViewMode;
use pptx_compose_json::schemas::{
    PatchStatus, Severity as JsonSeverity, ValidationReport,
    ValidationStatus as JsonValidationStatus,
};
use serde_json::Value;

#[path = "../../pptx-compose-core/tests/support/fixtures.rs"]
mod fixtures;

mod roundtrip_golden {
    use super::*;

    pub mod roundtrip {
        use super::*;

        #[test]
        fn no_edit_byte_identity() -> Result<()> {
            let manifest = fixtures::load_manifest();
            let roundtrip_fixtures = manifest
                .entries
                .iter()
                .filter(|entry| entry.has_invariant("roundtrip"))
                .collect::<Vec<_>>();

            assert!(
                !roundtrip_fixtures.is_empty(),
                "fixture manifest must include at least one roundtrip fixture"
            );
            assert!(
                roundtrip_fixtures
                    .iter()
                    .any(|entry| entry.has_feature("mc-alternate-content")
                        && entry.has_feature("unknown-part")),
                "roundtrip fixtures must cover mc:AlternateContent and unknown parts"
            );

            for fixture in roundtrip_fixtures {
                assert_no_edit_roundtrip(fixture)?;
            }

            Ok(())
        }
    }
}

mod edits {
    use super::*;

    #[test]
    fn add_image_runs_against_corpus_media_fixture() -> Result<()> {
        let manifest = fixtures::load_manifest();
        let fixture = manifest
            .entries
            .iter()
            .find(|entry| entry.has_invariant("edit-add-image") && entry.has_feature("media"))
            .expect("fixture manifest includes an add_image corpus fixture");

        let input = std::fs::read(fixtures::fixture_path(&fixture.path)).map_err(|source| {
            Error::parse_error(format!("Could not read fixture {}.", fixture.path), source)
        })?;
        let mut document = PresentationDocument::from_bytes(&input)?;
        let validation = document.validate()?;
        let patch = parse_patch(serde_json::json!({
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": validation.document_id,
            "base_revision": validation.revision,
            "client_request_id": "corpus-add-image",
            "operations": [{
                "operation_id": "add-corpus-image",
                "op": "add_image",
                "slide_id": "slide-1",
                "media_ref": "corpus-image",
                "content_type": "image/png",
                "bounds": { "x": 457200, "y": 3657600, "cx": 914400, "cy": 914400 }
            }]
        }))?;

        let report = document.apply_patch(patch, media_inputs("corpus-image", "image/png"))?;
        assert!(
            report
                .changed_parts
                .iter()
                .any(|part| part.starts_with("ppt/media/image") && part.ends_with(".png")),
            "add_image must create a media part"
        );
        assert!(
            report
                .changed_parts
                .iter()
                .any(|part| part == "ppt/slides/slide1.xml"),
            "add_image must update the slide XML"
        );

        let written = document.write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })?;
        let reopened = PresentationDocument::from_bytes(&written)?;
        assert_eq!(reopened.validate()?.status, JsonValidationStatus::Valid);

        let entries = from_bytes(&written)?;
        let content_types = entries
            .iter()
            .find(|entry| entry.name.zip_entry_name() == "[Content_Types].xml")
            .expect("written package contains content types");
        let content_types = String::from_utf8_lossy(&content_types.bytes);
        assert!(content_types.contains("image/png"));

        let slide_rels = entries
            .iter()
            .find(|entry| entry.name.zip_entry_name() == "ppt/slides/_rels/slide1.xml.rels")
            .expect("written package contains slide relationships");
        let slide_rels = String::from_utf8_lossy(&slide_rels.bytes);
        assert!(slide_rels.contains("relationships/image"));

        Ok(())
    }

    #[test]
    fn edited_real_world_fixture_preserves_unrelated_package_parts() -> Result<()> {
        let manifest = fixtures::load_manifest();
        let fixture = manifest
            .entries
            .iter()
            .find(|entry| {
                entry.path == "real-world/worldbank-cpf-concept-note.pptx"
                    && entry.has_feature("real-world")
                    && entry.has_invariant("roundtrip")
            })
            .expect("manifest includes the real-world preservation fixture");

        let input = std::fs::read(fixtures::fixture_path(&fixture.path)).map_err(|source| {
            Error::parse_error(format!("Could not read fixture {}.", fixture.path), source)
        })?;
        let original_entries = from_bytes(&input)?;
        let mut document = PresentationDocument::from_bytes(&input)?;
        let validation = document.validate()?;
        assert_valid_facade_validation(&validation, fixture);

        let view = document.to_agent_json_with_options(AgentViewOptions {
            mode: ViewMode::SlideDetail,
            include_elements: true,
            slide_id: Some("slide-3".to_owned()),
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        })?;
        let target = first_editable_text_run(&view)
            .expect("real-world fixture slide exposes an editable text run");
        let marker = "PPTX_COMPOSE_REAL_WORLD_PRESERVE_GATE";
        let mut selector = serde_json::json!({
            "type": "element_id",
            "id": target.element_id,
            "guards": {
                "slide_id": target.slide_id,
                "kind": target.kind,
                "part": target.part,
                "text_hash": target.element_text_hash,
                "fingerprint": target.fingerprint
            },
            "run": {
                "paragraph_index": target.paragraph_index,
                "run_index": target.run_index
            }
        });
        if let Some(run_text_hash) = &target.run_text_hash {
            selector["run"]["text_hash"] = serde_json::json!(run_text_hash);
        }
        let patch = parse_patch(serde_json::json!({
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": validation.document_id,
            "base_revision": validation.revision,
            "client_request_id": "real-world-preservation-gate",
            "operations": [{
                "operation_id": "replace-real-world-run",
                "op": "replace_text",
                "mode": "run_scoped",
                "selector": selector,
                "text": format!("{marker} replaced text")
            }]
        }))?;

        let report = document.apply_patch(patch, MediaInputs::default())?;
        assert_eq!(report.status, PatchStatus::Applied);
        assert_eq!(
            report.changed_parts,
            vec![target.part.clone()],
            "replace_text should dirty only the target slide part"
        );

        let written = document.write_vec_with_options(WriteOptions {
            mode: WriteMode::Preserve,
            ..WriteOptions::default()
        })?;
        let reopened = PresentationDocument::from_bytes(&written)?;
        assert_valid_facade_validation(&reopened.validate()?, fixture);

        let written_entries = from_bytes(&written)?;
        assert_equal_part_sets(&original_entries, &written_entries, &fixture.path);
        assert_important_unrelated_parts_preserved(
            &original_entries,
            &written_entries,
            &BTreeSet::from([target.part.clone()]),
            &fixture.path,
        );
        assert_part_contains(&written_entries, &target.part, marker, &fixture.path);

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditableTextRun {
    slide_id: String,
    element_id: String,
    kind: String,
    part: String,
    fingerprint: String,
    element_text_hash: String,
    paragraph_index: usize,
    run_index: usize,
    run_text_hash: Option<String>,
}

fn assert_no_edit_roundtrip(fixture: &fixtures::FixtureEntry) -> Result<()> {
    let path = fixtures::fixture_path(&fixture.path);
    let input = std::fs::read(&path).map_err(|source| {
        Error::parse_error(
            format!("Could not read fixture {}.", path.display()),
            source,
        )
    })?;
    let original_entries = from_bytes(&input)?;
    let original_package = fixtures::package_from_entries(&original_entries)?;
    assert_valid_no_edit_package(&original_package, fixture);

    let document = PresentationDocument::from_bytes(input.clone())?;
    assert_valid_facade_validation(&document.validate()?, fixture);
    let output = document.write_vec_with_options(WriteOptions {
        mode: WriteMode::Preserve,
        ..WriteOptions::default()
    })?;

    let reopened = PresentationDocument::from_bytes(output.clone())?;
    assert_valid_facade_validation(&reopened.validate()?, fixture);
    let written_entries = from_bytes(&output)?;
    let written_package = fixtures::package_from_entries(&written_entries)?;
    assert_valid_no_edit_package(&written_package, fixture);
    assert_equal_part_sets(&original_entries, &written_entries, &fixture.path);
    assert_byte_identical_parts(&original_entries, &written_entries, &fixture.path);

    Ok(())
}

fn assert_valid_no_edit_package(package: &Package, fixture: &fixtures::FixtureEntry) {
    let validation = validate_package(package, ValidationMode::NoEdit);
    assert_eq!(
        validation.status,
        ValidationStatus::Valid,
        "{}: no-edit package validation failed: {:#?}",
        fixture.path,
        validation.findings
    );
    fixtures::assert_expected_warnings(
        &fixture.path,
        &fixture.expected_warnings,
        validation
            .findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .map(|finding| finding_code_to_string(finding.code)),
    );
}

fn assert_valid_facade_validation(report: &ValidationReport, fixture: &fixtures::FixtureEntry) {
    assert_eq!(
        report.status,
        JsonValidationStatus::Valid,
        "{}: facade validation failed: {:#?}",
        fixture.path,
        report.findings
    );
    fixtures::assert_expected_warnings(
        &fixture.path,
        &fixture.expected_warnings,
        report
            .findings
            .iter()
            .filter(|finding| finding.severity == JsonSeverity::Warning)
            .map(|finding| finding_code_to_string(finding.code)),
    );
}

fn finding_code_to_string<T>(code: T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unserializable_finding_code".to_owned())
}

fn assert_equal_part_sets(
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    fixture: &str,
) {
    let original_names = part_names(original_entries);
    let written_names = part_names(written_entries);

    assert_eq!(
        written_names, original_names,
        "{fixture}: written package must contain exactly the original part set"
    );
}

fn assert_byte_identical_parts(
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    fixture: &str,
) {
    let written_by_name = written_entries
        .iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    for original_entry in original_entries {
        let written_entry = written_by_name
            .get(&original_entry.name)
            .unwrap_or_else(|| {
                panic!("{fixture}: written package dropped {}", original_entry.name)
            });
        assert_eq!(
            written_entry.bytes, original_entry.bytes,
            "{fixture}: part {} changed in a no-edit round trip",
            original_entry.name
        );
    }
}

fn assert_important_unrelated_parts_preserved(
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    changed_parts: &BTreeSet<String>,
    fixture: &str,
) {
    let written_by_name = written_entries
        .iter()
        .map(|entry| (entry.name.zip_entry_name().to_owned(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut checked = 0usize;

    for original_entry in original_entries {
        let name = original_entry.name.zip_entry_name();
        if changed_parts.contains(name) || !is_important_preservation_part(name) {
            continue;
        }

        checked += 1;
        let written_entry = written_by_name
            .get(name)
            .unwrap_or_else(|| panic!("{fixture}: written package dropped important part {name}"));
        assert_eq!(
            written_entry.bytes, original_entry.bytes,
            "{fixture}: edited output changed unrelated important part {name}"
        );
    }

    assert!(
        checked > 0,
        "{fixture}: preservation gate did not find any important unrelated parts to check"
    );
}

fn is_important_preservation_part(name: &str) -> bool {
    name == "[Content_Types].xml"
        || name.ends_with(".rels")
        || name.starts_with("ppt/media/")
        || name.starts_with("ppt/charts/")
        || name.starts_with("ppt/embeddings/")
        || name.starts_with("ppt/diagrams/")
        || name.contains("/embeddings/")
}

fn assert_part_contains(entries: &[RawEntry], part: &str, needle: &str, fixture: &str) {
    let entry = entries
        .iter()
        .find(|entry| entry.name.zip_entry_name() == part)
        .unwrap_or_else(|| panic!("{fixture}: written package is missing edited part {part}"));
    let text = String::from_utf8_lossy(&entry.bytes);
    assert!(
        text.contains(needle),
        "{fixture}: edited part {part} does not contain marker {needle}"
    );
}

fn first_editable_text_run(view: &Value) -> Option<EditableTextRun> {
    for slide in value_array(view.get("slides")) {
        for element in value_array(slide.get("elements")) {
            if !text_editable(element) {
                continue;
            }
            let text = element.get("text")?;
            for (paragraph_index, paragraph) in
                value_array(text.get("paragraphs")).iter().enumerate()
            {
                for (run_index, run) in value_array(paragraph.get("runs")).iter().enumerate() {
                    if string_field(run, "text").is_none_or(str::is_empty) {
                        continue;
                    }
                    return Some(EditableTextRun {
                        slide_id: string_field(element, "slide_id")?.to_owned(),
                        element_id: string_field(element, "id")?.to_owned(),
                        kind: string_field(element, "kind")?.to_owned(),
                        part: string_field(element, "part")?.to_owned(),
                        fingerprint: string_field(element, "fingerprint")?.to_owned(),
                        element_text_hash: string_field(text, "text_hash")?.to_owned(),
                        paragraph_index,
                        run_index,
                        run_text_hash: string_field(run, "text_hash").map(str::to_owned),
                    });
                }
            }
        }
    }

    None
}

fn text_editable(element: &Value) -> bool {
    element
        .get("editable")
        .and_then(|editable| editable.get("text"))
        .and_then(|text| text.get("supported"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn value_array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn part_names(entries: &[RawEntry]) -> BTreeSet<PartName> {
    entries.iter().map(|entry| entry.name.clone()).collect()
}

fn media_inputs(media_ref: &str, content_type: &str) -> MediaInputs {
    MediaInputs::new(HashMap::from([(
        media_ref.to_owned(),
        MediaBinding {
            content_type: content_type.to_owned(),
            declared_sha256: None,
            declared_byte_length: None,
            source: MediaSource::Bytes(tiny_png()),
        },
    )]))
}

fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xa7, 0x35, 0x81, 0xe9, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}
