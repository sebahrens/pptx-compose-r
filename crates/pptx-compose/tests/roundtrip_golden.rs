use std::collections::{BTreeMap, BTreeSet, HashMap};

use pptx_compose::{
    MediaInputs, PresentationDocument, WriteMode, WriteOptions,
    core::{
        error::{Error, Result},
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        validation::{ValidationMode, ValidationStatus, validate_package},
        xml::{document::XmlElement, parser::parse_document},
        zip::reader::{RawEntry, from_bytes},
    },
    edit::{
        media_inputs::{MediaBinding, MediaSource},
        patch::parse_patch,
    },
    json::schemas::ValidationStatus as JsonValidationStatus,
};

#[path = "../../pptx-compose-core/tests/support/fixtures.rs"]
mod fixtures;

mod roundtrip {
    use super::*;

    #[test]
    fn no_edit_byte_identity() -> Result<()> {
        let manifest = fixtures::load_manifest();
        let roundtrip_fixtures = manifest
            .entries
            .iter()
            .filter(|entry| entry.invariants.iter().any(|item| item == "roundtrip"))
            .collect::<Vec<_>>();

        assert!(
            !roundtrip_fixtures.is_empty(),
            "fixture manifest must include at least one roundtrip fixture"
        );
        assert!(
            roundtrip_fixtures.iter().any(|entry| entry
                .features
                .iter()
                .any(|feature| feature == "mc-alternate-content")
                && entry
                    .features
                    .iter()
                    .any(|feature| feature == "unknown-part")),
            "roundtrip fixtures must cover mc:AlternateContent and unknown parts"
        );

        for fixture in roundtrip_fixtures {
            assert_no_edit_roundtrip(fixture.path.as_str())?;
        }

        Ok(())
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
            .find(|entry| {
                entry.invariants.iter().any(|item| item == "edit-add-image")
                    && entry.features.iter().any(|feature| feature == "media")
            })
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
}

fn assert_no_edit_roundtrip(relative_path: &str) -> Result<()> {
    let path = fixtures::fixture_path(relative_path);
    let input = std::fs::read(&path).map_err(|source| {
        Error::parse_error(
            format!("Could not read fixture {}.", path.display()),
            source,
        )
    })?;
    let original_entries = from_bytes(&input)?;
    let original_package = package_from_entries(&original_entries)?;
    assert_valid_no_edit_package(&original_package, relative_path);

    let document = PresentationDocument::from_bytes(input.clone())?;
    let output = document.write_vec_with_options(WriteOptions {
        mode: WriteMode::Preserve,
        ..WriteOptions::default()
    })?;

    let written_entries = from_bytes(&output)?;
    let written_package = package_from_entries(&written_entries)?;
    assert_valid_no_edit_package(&written_package, relative_path);
    assert_equal_part_sets(&original_entries, &written_entries, relative_path);
    assert_byte_identical_parts(&original_entries, &written_entries, relative_path);

    Ok(())
}

fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        package.insert_part(pptx_compose::core::opc::part::Part::from_zip_entry(
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
        )?)?;
    }

    hydrate_content_types(&mut package)?;
    hydrate_relationships(&mut package)?;

    Ok(package)
}

fn hydrate_content_types(package: &mut Package) -> Result<()> {
    let content_types_name = PartName::from_zip_entry("[Content_Types].xml")?;
    let raw = package
        .parts()
        .get(&content_types_name)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?
        .bytes()
        .to_vec();
    let document = parse_document(&raw)?;
    let root = document
        .root_element()
        .ok_or_else(|| Error::unsupported_package("[Content_Types].xml has no root element."))?;

    if root.name.local_name != "Types" {
        return Err(Error::unsupported_package(
            "[Content_Types].xml root element is not Types.",
        ));
    }

    for child in root.children.iter().filter_map(|node| node.as_element()) {
        match child.name.local_name.as_str() {
            "Default" => {
                let extension = required_attr(child, "Extension")?;
                let content_type = required_attr(child, "ContentType")?;
                package
                    .content_types_mut()
                    .insert_default(extension, content_type);
            }
            "Override" => {
                let part_name = PartName::from_zip_entry(required_attr(child, "PartName")?)?;
                let content_type = required_attr(child, "ContentType")?;
                package
                    .content_types_mut()
                    .insert_override(part_name, content_type);
            }
            _ => {}
        }
    }

    Ok(())
}

fn hydrate_relationships(package: &mut Package) -> Result<()> {
    let rels_entries = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part_name, raw) in rels_entries {
        let source = relationship_source_for(&rels_part_name)?;
        let document = parse_document(&raw)?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::unsupported_package(".rels part has no root element."))?;

        if root.name.local_name != "Relationships" {
            return Err(Error::unsupported_package(
                ".rels root element is not Relationships.",
            ));
        }

        for child in root.children.iter().filter_map(|node| node.as_element()) {
            if child.name.local_name != "Relationship" {
                continue;
            }

            let id = required_attr(child, "Id")?;
            let rel_type = required_attr(child, "Type")?;
            let target = required_attr(child, "Target")?;
            let relationship = if optional_attr(child, "TargetMode") == Some("External") {
                Relationship::external(source.clone(), id, rel_type, target)
            } else {
                Relationship::internal(source.clone(), id, rel_type, target)
            };
            package.push_relationship(relationship);
        }
    }

    Ok(())
}

fn relationship_source_for(rels_part_name: &PartName) -> Result<RelationshipSource> {
    let rels_path = rels_part_name.as_str();
    if rels_path == "/_rels/.rels" {
        return Ok(RelationshipSource::Package);
    }

    let Some((directory, file_name)) = rels_path.rsplit_once("/_rels/") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} is not in an _rels directory."
        )));
    };
    let Some(source_file_name) = file_name.strip_suffix(".rels") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} does not end with .rels."
        )));
    };

    PartName::from_zip_entry(format!("{directory}/{source_file_name}").as_str())
        .map(RelationshipSource::Part)
}

fn required_attr<'a>(element: &'a XmlElement, name: &str) -> Result<&'a str> {
    optional_attr(element, name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Element {} is missing required attribute {name}.",
            element.name.raw
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

fn assert_valid_no_edit_package(package: &Package, fixture: &str) {
    let validation = validate_package(package, ValidationMode::NoEdit);
    assert_eq!(
        validation.status,
        ValidationStatus::Valid,
        "{fixture}: no-edit package validation failed: {:#?}",
        validation.findings
    );
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
