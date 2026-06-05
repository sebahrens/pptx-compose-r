use std::{collections::BTreeMap, path::Path};

use pptx_compose_core::{
    error::{Error, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{Relationship, RelationshipSource},
    },
    validation::{ValidationMode, ValidationStatus, validate_package},
    xml::{document::XmlElement, parser::parse_document},
    zip::{
        reader::{RawEntry, from_bytes},
        writer::{WriteEntry, write_vec},
    },
};

fn assert_no_edit_roundtrip(path: &Path) -> Result<()> {
    let input = std::fs::read(path).map_err(|source| {
        Error::parse_error(
            format!("Could not read fixture {}.", path.display()),
            source,
        )
    })?;
    let original_entries = from_bytes(&input)?;
    let original_package = package_from_entries(&original_entries)?;

    assert_valid_no_edit_package(&original_package);

    let write_entries: Vec<_> = original_entries.iter().map(WriteEntry::Clean).collect();
    let output = write_vec(&input, &write_entries)?;
    let written_entries = from_bytes(&output)?;
    let written_package = package_from_entries(&written_entries)?;

    assert_valid_no_edit_package(&written_package);
    assert_byte_identical_parts(&original_entries, &written_entries);

    Ok(())
}

#[test]
fn no_edit_byte_identity() -> Result<()> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/minimal.pptx")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy/sample.pptx")
        });

    assert_no_edit_roundtrip(&fixture)
}

fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        package.insert_part(pptx_compose_core::opc::part::Part::from_zip_entry(
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
    let rels_entries: Vec<_> = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect();

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

fn assert_valid_no_edit_package(package: &Package) {
    let validation = validate_package(package, ValidationMode::NoEdit);
    assert_eq!(
        validation.status,
        ValidationStatus::Valid,
        "no-edit package validation failed: {:#?}",
        validation.findings
    );
}

fn assert_byte_identical_parts(original_entries: &[RawEntry], written_entries: &[RawEntry]) {
    let written_by_name: BTreeMap<_, _> = written_entries
        .iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect();

    assert_eq!(
        written_entries.len(),
        original_entries.len(),
        "written package must contain the same number of parts"
    );

    for original_entry in original_entries {
        let written_entry = written_by_name
            .get(&original_entry.name)
            .unwrap_or_else(|| panic!("written package dropped {}", original_entry.name));
        assert_eq!(
            written_entry.bytes, original_entry.bytes,
            "part {} changed in a no-edit round trip",
            original_entry.name
        );
    }
}
